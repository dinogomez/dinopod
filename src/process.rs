//! Host dev process spawn, stop, and log tracking for native mode.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cmd::{path_display, CommandRunner, CommandSpec};
use crate::detect::PackageManager;
use crate::env::{env_overlay_path, install_program, load_merged_env, StdEnvFs};
use crate::errors::{DinopodError, Result};

/// Launch parameters for a native dev process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeDevLaunch {
    /// Worktree root where the dev script should run.
    pub worktree_root: PathBuf,
    /// Detected package manager.
    pub package_manager: PackageManager,
    /// npm/pnpm script name.
    pub script: String,
    /// Merged environment variables for the dev process.
    pub env: Vec<(String, String)>,
}

/// Paths for native dev process artifacts inside a worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevProcessPaths {
    /// PID file path.
    pub pid_file: PathBuf,
    /// Log file path.
    pub log_file: PathBuf,
}

impl DevProcessPaths {
    /// Returns native dev artifact paths under `.dinopod/`.
    #[must_use]
    pub fn new(worktree_root: &Path) -> Self {
        let dinopod = worktree_root.join(".dinopod");
        Self {
            pid_file: dinopod.join("dev.pid"),
            log_file: dinopod.join("dev.log"),
        }
    }
}

/// Filesystem boundary for process supervision.
pub trait ProcessFs {
    /// Returns true when `path` exists.
    fn path_exists(&self, path: &Path) -> bool;

    /// Reads a UTF-8 file.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;

    /// Writes a file with the requested Unix mode when supported.
    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> io::Result<()>;

    /// Removes a file when present.
    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Creates a directory and parents.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Returns true when the PID is alive.
    fn pid_is_alive(&self, pid: u32) -> bool;
}

/// Production filesystem adapter for process supervision.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdProcessFs;

impl ProcessFs for StdProcessFs {
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            self.create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        set_mode(path, mode)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)?;
        set_mode(path, 0o700)
    }

    fn pid_is_alive(&self, pid: u32) -> bool {
        process_is_alive(pid)
    }
}

/// Returns whether `pid` refers to a live process (probe only, no signals sent).
#[must_use]
pub fn process_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Builds a detached dev-process spawn command that prints the child PID.
///
/// # Errors
///
/// Returns [`DinopodError::DevProcessSpawnFailed`] when an environment key is not
/// a valid shell identifier.
pub fn build_spawn_command(
    worktree_root: &Path,
    package_manager: PackageManager,
    script: &str,
    log_file: &Path,
    env: &[(String, String)],
) -> Result<CommandSpec> {
    let script_command = script_invocation(package_manager, script);
    let log_path = shell_quote(&path_display(log_file));
    let exports = env
        .iter()
        .map(|(key, value)| shell_export(key, value))
        .collect::<Result<Vec<_>>>()?
        .join("; ");
    let prefix = if exports.is_empty() {
        String::new()
    } else {
        format!("{exports}; ")
    };
    let shell = format!(
        "{prefix}export HOSTNAME=0.0.0.0; nohup {script_command} >> {log_path} 2>&1 & echo $!"
    );

    Ok(CommandSpec::new("sh")
        .arg("-c")
        .arg(shell)
        .current_dir(worktree_root.to_path_buf()))
}

/// Spawns a detached dev script in the worktree.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when spawn fails.
pub fn spawn_dev_process<R: CommandRunner, F: ProcessFs>(
    runner: &R,
    fs: &F,
    worktree_root: &Path,
    package_manager: PackageManager,
    script: &str,
    env: &[(String, String)],
) -> Result<u32> {
    let paths = DevProcessPaths::new(worktree_root);
    fs.create_dir_all(
        paths
            .pid_file
            .parent()
            .expect("pid file should have a parent directory"),
    )
    .map_err(DinopodError::Io)?;

    let command = build_spawn_command(worktree_root, package_manager, script, &paths.log_file, env)?;
    let output = runner.run(&command)?;
    if !output.success() {
        return Err(DinopodError::DevProcessSpawnFailed {
            stderr: output.stderr().to_owned(),
        });
    }

    let pid =
        output
            .stdout()
            .trim()
            .parse::<u32>()
            .map_err(|_| DinopodError::DevProcessSpawnFailed {
                stderr: "spawn did not return a pid".to_owned(),
            })?;

    fs.write_file(&paths.pid_file, &pid.to_string(), 0o600)
        .map_err(DinopodError::Io)?;
    Ok(pid)
}

/// Runs the native dev script in the foreground with inherited terminal I/O.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when the process cannot be started,
/// or [`DinopodError::DevProcessExited`] when it exits with a non-zero status.
pub fn run_dev_process_foreground(launch: &NativeDevLaunch) -> Result<()> {
    let fs = StdProcessFs;
    let paths = DevProcessPaths::new(&launch.worktree_root);
    fs.create_dir_all(
        paths
            .pid_file
            .parent()
            .expect("pid file should have a parent directory"),
    )
    .map_err(DinopodError::Io)?;

    let mut command = build_foreground_dev_command(launch);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let mut child = command.spawn().map_err(DinopodError::Io)?;
    fs.write_file(&paths.pid_file, &child.id().to_string(), 0o600)
        .map_err(DinopodError::Io)?;

    let status = child.wait().map_err(DinopodError::Io)?;
    let _ = fs.remove_file(&paths.pid_file);

    if status.success() {
        Ok(())
    } else {
        Err(DinopodError::DevProcessExited {
            code: status.code(),
        })
    }
}

/// Formats a program plus args for status lines and logs.
#[must_use]
pub fn format_exec_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_owned()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

/// Runs a command in a worktree with explicit environment variables and inherited stdio.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when the command cannot be started.
pub fn exec_in_worktree_foreground(
    worktree_root: &Path,
    program: &str,
    args: &[String],
    env: &[(String, String)],
) -> Result<std::process::ExitStatus> {
    let mut command = Command::new(program);
    command
        .current_dir(worktree_root)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (key, value) in env {
        command.env(key, value);
    }

    command.status().map_err(DinopodError::Io)
}

/// Runs a command in a worktree with merged dotenv files and Dinopod overlay env.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when env files cannot be read or the process
/// cannot be started.
/// Captured output from a worktree command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedProcessOutput {
    /// Process exit status.
    pub status: std::process::ExitStatus,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

/// Runs a command in a worktree with explicit environment variables and captures output.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when the command cannot be started.
pub fn exec_in_worktree_with_env(
    worktree_root: &Path,
    program: &str,
    args: &[String],
    env: &[(String, String)],
) -> Result<CapturedProcessOutput> {
    let mut command = Command::new(program);
    command
        .current_dir(worktree_root)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }

    let output = command.output().map_err(DinopodError::Io)?;
    Ok(CapturedProcessOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Runs a command in a worktree with merged env, inheriting stdio.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when the command cannot be started.
pub fn exec_in_worktree(
    worktree_root: &Path,
    program: &str,
    args: &[String],
) -> Result<std::process::ExitStatus> {
    let overlay_path = env_overlay_path(worktree_root);
    let merged_env = load_merged_env(worktree_root, &overlay_path, &StdEnvFs)?;
    let env: Vec<(String, String)> = merged_env.into_iter().collect();
    let captured = exec_in_worktree_with_env(worktree_root, program, args, &env)?;
    if !captured.stdout.is_empty() {
        let _ = std::io::stdout().write_all(captured.stdout.as_bytes());
    }
    if !captured.stderr.is_empty() {
        let _ = std::io::stderr().write_all(captured.stderr.as_bytes());
    }
    Ok(captured.status)
}

fn build_foreground_dev_command(launch: &NativeDevLaunch) -> Command {
    let (program, args) = dev_command_parts(launch.package_manager, &launch.script);
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&launch.worktree_root)
        .env("HOSTNAME", "0.0.0.0");
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

fn dev_command_parts(package_manager: PackageManager, script: &str) -> (&'static str, Vec<String>) {
    match package_manager {
        PackageManager::Pnpm => (install_program(package_manager), vec![script.to_owned()]),
        PackageManager::Npm => (
            install_program(package_manager),
            vec!["run".to_owned(), script.to_owned()],
        ),
    }
}

/// Verifies a freshly spawned dev process is still alive.
///
/// # Errors
///
/// Returns [`DinopodError::DevProcessSpawnFailed`] when the process exits immediately,
/// including a tail of `.dinopod/dev.log` when available.
pub fn ensure_dev_process_running<F: ProcessFs>(
    fs: &F,
    worktree_root: &Path,
    pid: u32,
) -> Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(250));
    if fs.pid_is_alive(pid) {
        return Ok(());
    }

    let paths = DevProcessPaths::new(worktree_root);
    let stderr = if fs.path_exists(&paths.log_file) {
        fs.read_to_string(&paths.log_file).map_or_else(
            |_| "dev process exited immediately".to_owned(),
            |contents| tail_lines(&contents, 20),
        )
    } else {
        "dev process exited immediately".to_owned()
    };

    Err(DinopodError::DevProcessSpawnFailed { stderr })
}

fn tail_lines(contents: &str, max_lines: usize) -> String {
    contents
        .lines()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Stops a tracked dev process when alive.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when signal delivery fails unexpectedly.
pub fn stop_dev_process<F: ProcessFs>(fs: &F, worktree_root: &Path) -> Result<()> {
    let paths = DevProcessPaths::new(worktree_root);
    let Some(pid) = read_pid(fs, &paths.pid_file)? else {
        return Ok(());
    };

    terminate_process_tree(pid);
    fs.remove_file(&paths.pid_file).map_err(DinopodError::Io)
}

/// Stops any process listening on `port`, used before restarting native dev.
pub fn terminate_listeners_on_port(port: u16) {
    #[cfg(unix)]
    {
        let Ok(output) = Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
            .output()
        else {
            return;
        };

        for pid in parse_pids_from_output(&output.stdout) {
            terminate_process_tree(pid);
        }
    }

    #[cfg(not(unix))]
    let _ = port;
}

/// Returns the tracked PID when present.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when the PID file cannot be read.
pub fn read_dev_pid<F: ProcessFs>(fs: &F, worktree_root: &Path) -> Result<Option<u32>> {
    read_pid(fs, &DevProcessPaths::new(worktree_root).pid_file)
}

fn read_pid<F: ProcessFs>(fs: &F, pid_file: &Path) -> Result<Option<u32>> {
    if !fs.path_exists(pid_file) {
        return Ok(None);
    }
    let contents = fs.read_to_string(pid_file).map_err(DinopodError::Io)?;
    contents
        .trim()
        .parse::<u32>()
        .map(Some)
        .map_err(|_| DinopodError::DevProcessPidInvalid {
            contents: contents.trim().to_owned(),
        })
}

fn script_invocation(package_manager: PackageManager, script: &str) -> String {
    let (program, args) = dev_command_parts(package_manager, script);
    if args.len() == 1 {
        format!("{} {}", program, args[0])
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn is_shell_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn shell_export(key: &str, value: &str) -> Result<String> {
    if !is_shell_env_key(key) {
        return Err(DinopodError::DevProcessSpawnFailed {
            stderr: format!(
                "environment key `{key}` is not a valid shell identifier; rename it in your dotenv files"
            ),
        });
    }
    Ok(format!("export {key}={}", shell_quote(value)))
}

fn terminate_process_tree(pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("pkill")
            .args(["-TERM", "-P", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::thread::sleep(std::time::Duration::from_millis(200));
        if process_pid_is_alive(pid) {
            let _ = Command::new("pkill")
                .args(["-KILL", "-P", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    #[cfg(not(unix))]
    let _ = pid;
}

fn process_pid_is_alive(pid: u32) -> bool {
    process_is_alive(pid)
}

#[cfg(unix)]
fn parse_pids_from_output(stdout: &[u8]) -> Vec<u32> {
    stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_command_parts_should_map_pnpm_and_npm_scripts() {
        assert_eq!(
            dev_command_parts(PackageManager::Pnpm, "dev:all"),
            ("pnpm", vec!["dev:all".to_owned()])
        );
        assert_eq!(
            dev_command_parts(PackageManager::Npm, "dev"),
            ("npm", vec!["run".to_owned(), "dev".to_owned()])
        );
    }

    #[test]
    fn format_exec_command_should_join_program_and_args() {
        assert_eq!(
            format_exec_command("pnpm", &["dev:all".to_owned()]),
            "pnpm dev:all"
        );
    }

    #[test]
    fn shell_export_should_reject_invalid_env_keys() {
        let error = shell_export("FOO-BAR", "value").expect_err("hyphenated keys should fail");
        assert!(matches!(error, DinopodError::DevProcessSpawnFailed { .. }));
    }

    #[test]
    fn shell_export_should_quote_values() {
        assert_eq!(
            shell_export("DATABASE_URL", "postgres://host/db").expect("valid key"),
            "export DATABASE_URL='postgres://host/db'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn exec_in_worktree_should_apply_overlay_env() {
        use std::fs;

        let root =
            std::env::temp_dir().join(format!("dinopod-exec-env-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp worktree should be created");
        fs::create_dir_all(root.join(".dinopod")).expect("dinopod dir should be created");
        fs::write(
            root.join(".env"),
            "DATABASE_URL=postgresql://localhost:5432/app\n",
        )
        .expect(".env should be written");
        fs::write(
            root.join(".dinopod/env.overlay"),
            "DATABASE_URL=postgresql://localhost:54321/app\n",
        )
        .expect("overlay should be written");

        let status = exec_in_worktree(
            &root,
            "sh",
            &[
                "-c".to_owned(),
                "test \"$DATABASE_URL\" = \"postgresql://localhost:54321/app\"".to_owned(),
            ],
        )
        .expect("exec should run");

        assert!(status.success(), "overlay env should override copied .env");
        let _ = fs::remove_dir_all(root);
    }
}
