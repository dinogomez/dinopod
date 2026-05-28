use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{fs, process};

static CLI_FIXTURE_COUNTER: AtomicU32 = AtomicU32::new(0);

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn version_should_print_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("--version")
        .output()
        .expect("dinopod binary should run");

    let stdout = String::from_utf8(output.stdout).expect("version output should be valid UTF-8");

    assert!(
        output.status.success() && stdout.contains(env!("CARGO_PKG_VERSION")),
        "version should succeed and print the package version"
    );
}

#[test]
fn help_should_list_core_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("--help")
        .output()
        .expect("dinopod binary should run");

    let stdout = String::from_utf8(output.stdout).expect("help output should be valid UTF-8");

    assert!(
        output.status.success()
            && stdout.contains("new")
            && stdout.contains("init")
            && stdout.contains("list")
            && stdout.contains("stop")
            && stdout.contains("rm"),
        "help should succeed and list the core commands: {stdout}"
    );
}

#[test]
fn no_command_should_display_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .output()
        .expect("dinopod binary should run");

    let stdout = String::from_utf8(output.stdout).expect("help output should be valid UTF-8");

    assert!(
        output.status.success() && stdout.contains("Usage: dinopod") && stdout.contains("rm"),
        "no command should render help and exit successfully"
    );
    assert!(
        stdout.contains("░███████"),
        "no command should render the inline welcome banner"
    );
}

#[test]
fn init_should_write_default_config_in_current_directory() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-init-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("init")
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod init should run");

    let config = fs::read_to_string(temp_dir.join("dinopod.toml"))
        .expect("dinopod init should write config");

    assert!(output.status.success());
    assert!(config.contains("[compose]"));
    assert!(config.contains("[settings]"));
    assert!(config.contains("image = \"traefik:v3.6\""));
}

#[test]
fn init_should_not_overwrite_existing_config() {
    let temp_dir =
        std::env::temp_dir().join(format!("dinopod-cli-init-existing-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    fs::write(temp_dir.join("dinopod.toml"), "existing config\n")
        .expect("existing config should be written");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("init")
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod init should run");

    let config = fs::read_to_string(temp_dir.join("dinopod.toml"))
        .expect("dinopod config should remain readable");

    assert!(!output.status.success());
    assert_eq!(config, "existing config\n");
}

#[test]
fn dev_should_report_missing_git_before_trying_lifecycle_commands() {
    let temp_dir =
        std::env::temp_dir().join(format!("dinopod-cli-preflight-test-{}", process::id()));
    let bin_dir = temp_dir.join("bin");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&bin_dir).expect("temp bin dir should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("dev")
        .arg("JIRA-123")
        .env("PATH", &bin_dir)
        .env("DINOPOD_CONFIG_DIR", temp_dir.join("config"))
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod dev should run");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(!output.status.success());
    assert!(stderr.contains("missing required dependency: git"));
}

#[cfg(unix)]
#[test]
fn new_from_subdirectory_should_use_primary_repo_root_and_root_config() {
    let fixture = FakeCliRepo::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("new")
        .arg("JIRA-123")
        .env("PATH", fixture.path_env())
        .env("DINOPOD_CONFIG_DIR", &fixture.config_root)
        .env("DINOPOD_FAKE_LOG", &fixture.fake_log)
        .env("DINOPOD_REPO_ROOT", &fixture.repo)
        .current_dir(&fixture.subdir)
        .output()
        .expect("dinopod new should run");

    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(output.status.success(), "stderr: {stderr}");
    assert!(stdout.contains("  🦕 project  myrepo-jira-123"));
    assert!(stdout.contains(&format!(
        "  🦕 url  http://jira-123-myrepo.localhost:{}",
        FakeCliRepo::http_port()
    )));
    assert!(stderr.contains("fixed host port") || stdout.contains("  🦕 worktree  "));
    let log = fs::read_to_string(&fixture.fake_log).expect("fake command log should be readable");
    assert!(log.contains(&format!(
        "git worktree add -b jira-123 {}",
        fixture.worktree_root.join("myrepo-jira-123").display()
    )));
}

#[test]
fn list_with_empty_state_should_succeed_without_docker() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-list-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("list")
        .env("DINOPOD_CONFIG_DIR", &temp_dir)
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod list should run");

    assert!(output.status.success());
}

#[test]
fn exec_should_require_tracked_environment() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-exec-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&temp_dir)
        .output()
        .expect("git init should succeed");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .args(["number-123", "echo", "hello"])
        .env("DINOPOD_CONFIG_DIR", &temp_dir)
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod exec should run");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(!output.status.success());
    assert!(stderr.contains("environment is not tracked: number-123"));
}

#[test]
fn rm_without_yes_should_fail_before_removing_when_not_tty() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-rm-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("rm")
        .arg("number-123")
        .env("DINOPOD_CONFIG_DIR", &temp_dir)
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod rm should run");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(!output.status.success());
    assert!(
        stderr.contains("confirmation required before removing environment: number-123")
            || stderr.contains("missing required dependency: git")
            || stderr.contains("environment is not tracked: number-123")
            || stderr.contains("not inside a Git repository"),
        "unexpected stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn rm_without_yes_should_require_confirmation_for_tracked_environment_when_not_tty() {
    let fixture = FakeCliRepo::new();
    let new_output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .args(["new", "JIRA-123"])
        .env("PATH", fixture.path_env())
        .env("DINOPOD_CONFIG_DIR", &fixture.config_root)
        .env("DINOPOD_FAKE_LOG", &fixture.fake_log)
        .env("DINOPOD_REPO_ROOT", &fixture.repo)
        .current_dir(&fixture.subdir)
        .output()
        .expect("dinopod new should run");
    assert!(
        new_output.status.success(),
        "dinopod new should succeed: {}",
        String::from_utf8_lossy(&new_output.stderr)
    );

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .args(["rm", "JIRA-123"])
        .env("PATH", fixture.path_env())
        .env("DINOPOD_CONFIG_DIR", &fixture.config_root)
        .env("DINOPOD_FAKE_LOG", &fixture.fake_log)
        .env("DINOPOD_REPO_ROOT", &fixture.repo)
        .current_dir(&fixture.subdir)
        .output()
        .expect("dinopod rm should run");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(!output.status.success());
    assert!(
        stderr.contains("confirmation required before removing environment: JIRA-123"),
        "unexpected stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn rm_from_inside_worktree_should_warn_to_use_main_repo() {
    let fixture = FakeCliRepo::new();
    let new_output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .args(["new", "JIRA-123"])
        .env("PATH", fixture.path_env())
        .env("DINOPOD_CONFIG_DIR", &fixture.config_root)
        .env("DINOPOD_FAKE_LOG", &fixture.fake_log)
        .env("DINOPOD_REPO_ROOT", &fixture.repo)
        .current_dir(&fixture.subdir)
        .output()
        .expect("dinopod new should run");
    assert!(
        new_output.status.success(),
        "dinopod new should succeed: {}",
        String::from_utf8_lossy(&new_output.stderr)
    );

    let worktree = fixture.worktree_root.join("myrepo-jira-123");
    assert!(
        worktree.is_dir(),
        "worktree directory should exist: {}",
        worktree.display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .args(["rm", "JIRA-123", "--yes"])
        .env("PATH", fixture.path_env())
        .env("DINOPOD_CONFIG_DIR", &fixture.config_root)
        .env("DINOPOD_FAKE_LOG", &fixture.fake_log)
        .env("DINOPOD_REPO_ROOT", &fixture.repo)
        .current_dir(&worktree)
        .output()
        .expect("dinopod rm should run");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(
        stderr.contains("current directory is inside the worktree being removed"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn list_should_succeed_when_lifecycle_lock_is_held() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-lock-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    fs::write(temp_dir.join("dinopod.lock"), "pid=999999\ncreated_at_unix_seconds=0\n")
        .expect("lock file should be written");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("list")
        .env("DINOPOD_CONFIG_DIR", &temp_dir)
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod list should run");

    assert!(
        output.status.success(),
        "read-only list should ignore an existing guard file: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn list_reconcile_should_fail_when_lifecycle_lock_is_held() {
    let temp_dir =
        std::env::temp_dir().join(format!("dinopod-cli-lock-reconcile-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let lock_path = temp_dir.join("dinopod.lock");
    let lock_path_arg = lock_path.display().to_string();
    let mut holder = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "printf 'pid=%s\\ncreated_at_unix_seconds=%s\\ntoken=held\\n' \"$$\" \"$(date +%s)\" > '{lock_path_arg}' && exec sleep 120"
        ))
        .spawn()
        .expect("lock holder should start");
    std::thread::sleep(std::time::Duration::from_millis(100));

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .args(["list", "--reconcile"])
        .env("DINOPOD_CONFIG_DIR", &temp_dir)
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod list --reconcile should run");

    let _ = holder.kill();
    let _ = holder.wait();

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(!output.status.success());
    assert!(stderr.contains("another dinopod command is already running"));
}

#[cfg(unix)]
#[derive(Debug)]
struct FakeCliRepo {
    repo: std::path::PathBuf,
    subdir: std::path::PathBuf,
    fake_bin: std::path::PathBuf,
    fake_log: std::path::PathBuf,
    config_root: std::path::PathBuf,
    worktree_root: std::path::PathBuf,
}

#[cfg(unix)]
impl FakeCliRepo {
    fn new() -> Self {
        let n = CLI_FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "dinopod-cli-subdir-repo-test-{}-{}",
            process::id(),
            n
        ));
        let repo = root.join("myrepo");
        let fixture = Self {
            subdir: repo.join("src"),
            fake_bin: root.join("bin"),
            fake_log: root.join("commands.log"),
            config_root: root.join("config"),
            worktree_root: root.join("worktrees"),
            repo,
        };
        fixture.write_files(&root);
        fixture
    }

    fn path_env(&self) -> String {
        format!(
            "{}:{}",
            self.fake_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn http_port() -> u16 {
        18000 + (process::id() % 1000) as u16
    }

    fn write_files(&self, root: &std::path::Path) {
        let _ = fs::remove_dir_all(root);
        fs::create_dir_all(&self.subdir).expect("repo subdir should be created");
        fs::create_dir_all(&self.fake_bin).expect("fake bin should be created");
        fs::write(
            self.repo.join("docker-compose.yml"),
            "services:\n  app:\n    image: example\n",
        )
        .expect("compose fixture should be written");
        fs::write(self.repo.join("dinopod.toml"), self.config())
            .expect("config fixture should be written");
        write_executable(&self.fake_bin.join("git"), FAKE_GIT);
        write_executable(&self.fake_bin.join("docker"), FAKE_DOCKER);
    }

    fn config(&self) -> String {
        format!(
            concat!(
                "[app]\n",
                "service = \"app\"\n",
                "internal_port = 3000\n",
                "compose_file = \"docker-compose.yml\"\n",
                "default_branch = \"main\"\n\n",
                "[worktree]\n",
                "root = \"{}\"\n\n",
                "[proxy]\n",
                "host_suffix = \"localhost\"\n",
                "network = \"dinopod-test-proxy\"\n",
                "container_name = \"dinopod-test-traefik\"\n",
                "http_port = {}\n",
                "image = \"traefik:v3.6\"\n",
            ),
            self.worktree_root.display(),
            Self::http_port()
        )
    }
}

#[cfg(unix)]
const FAKE_GIT: &str = r#"#!/bin/sh
printf 'git %s\n' "$*" >> "$DINOPOD_FAKE_LOG"
if [ "$1" = "--version" ]; then
  echo "git version fake"
  exit 0
fi
if [ "$1" = "rev-parse" ] && [ "$2" = "--is-inside-work-tree" ]; then
  echo "true"
  exit 0
fi
if [ "$1" = "worktree" ] && [ "$2" = "list" ]; then
  echo "worktree $DINOPOD_REPO_ROOT"
  echo "branch refs/heads/main"
  exit 0
fi
if [ "$1" = "rev-parse" ] && [ "$2" = "--verify" ]; then
  exit 1
fi
if [ "$1" = "worktree" ] && [ "$2" = "add" ]; then
  mkdir -p "$5"
  cp "$DINOPOD_REPO_ROOT/docker-compose.yml" "$5/docker-compose.yml"
  exit 0
fi
exit 0
"#;

#[cfg(unix)]
const FAKE_DOCKER: &str = r#"#!/bin/sh
printf 'docker %s\n' "$*" >> "$DINOPOD_FAKE_LOG"
if [ "$1" = "--version" ] || [ "$1" = "info" ]; then
  exit 0
fi
if [ "$1" = "compose" ] && [ "$2" = "version" ]; then
  exit 0
fi
if [ "$1" = "ps" ]; then
  exit 0
fi
if [ "$1" = "inspect" ]; then
  exit 1
fi
if [ "$1" = "network" ] && [ "$2" = "inspect" ]; then
  exit 1
fi
if [ "$1" = "network" ] && [ "$2" = "create" ]; then
  exit 0
fi
for arg in "$@"; do
  if [ "$arg" = "config" ]; then
    echo '{"services":{"app":{"ports":[{"target":3000,"published":"3000"}],"networks":{}}}}'
    exit 0
  fi
  if [ "$arg" = "up" ]; then
    exit 0
  fi
done
exit 0
"#;

#[cfg(unix)]
fn write_executable(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).expect("fake executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("fake executable metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("fake executable should be executable");
}
