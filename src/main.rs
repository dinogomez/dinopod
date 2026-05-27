#![forbid(unsafe_code)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use dinopod::cli::{Cli, Command};
use dinopod::cmd::StdCommandRunner;
use dinopod::config::{render_starter_config, DinopodConfig};
use dinopod::errors::{DinopodError, Result};
use dinopod::git::{GitWorktreeManager, StdWorktreeFs};
use dinopod::lifecycle::LifecycleManager;
use dinopod::lock::FileLock;
use dinopod::preflight::{CommandPreflightProbe, Dependency, PreflightChecker};
use dinopod::proxy::ProxyPaths;
use dinopod::runtime::CommandLifecyclePorts;
use dinopod::state::FileStateStore;
use dinopod::ui::{TerminalUi, Ui};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dinopod: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let Some(command) = cli.command else {
        let mut command = Cli::command();
        command.print_help()?;
        println!();
        return Ok(());
    };

    match command {
        Command::Init => init(Path::new("dinopod.toml")),
        Command::Dev { ticket } => with_lifecycle(LifecycleMode::Dev, |manager| {
            let mut ui = TerminalUi;
            let summary = manager.dev(&ticket)?;
            for warning in &summary.warnings {
                ui.warning(&warning.to_string())?;
            }
            ui.success(&format!("worktree: {}", summary.worktree_path.display()))?;
            ui.success(&format!("project: {}", summary.project))?;
            ui.success(&format!("url: {}", summary.url))?;
            Ok(())
        }),
        Command::List => with_lifecycle(LifecycleMode::List, |manager| {
            let mut ui = TerminalUi;
            for record in manager.list()? {
                ui.status(&format!(
                    "{}\t{:?}\t{}",
                    record.project, record.status, record.url
                ))?;
            }
            Ok(())
        }),
        Command::Stop { ticket } => {
            with_lifecycle(LifecycleMode::Mutating, |manager| manager.stop(&ticket))
        }
        Command::Down { ticket, volumes } => with_lifecycle(LifecycleMode::Mutating, |manager| {
            manager.down(&ticket, volumes)
        }),
        Command::Rm { ticket, yes } => {
            with_lifecycle(LifecycleMode::Mutating, |manager| manager.rm(&ticket, yes))
        }
    }
}

fn init(path: &Path) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                DinopodError::ConfigAlreadyExists {
                    path: path.to_path_buf(),
                }
            } else {
                DinopodError::from(error)
            }
        })?;
    file.write_all(render_starter_config(&DinopodConfig::default()).as_bytes())?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleMode {
    Dev,
    Mutating,
    List,
}

fn with_lifecycle<T>(
    mode: LifecycleMode,
    operation: impl FnOnce(
        &LifecycleManager<'_, CommandLifecyclePorts<StdCommandRunner>, FileStateStore>,
    ) -> Result<T>,
) -> Result<T> {
    let current_dir = std::env::current_dir()?;
    let runner = StdCommandRunner;
    let (repo_root, repo_name, config) = lifecycle_context(mode, runner, &current_dir)?;
    let config_root = config_root();
    let lock_path = config_root.join("dinopod.lock");
    let Some(_lock) = FileLock::try_acquire(&lock_path)? else {
        return Err(DinopodError::LockUnavailable { path: lock_path });
    };
    let proxy_paths = ProxyPaths::new(&config_root);
    let ports = CommandLifecyclePorts::new(StdCommandRunner, config.clone(), proxy_paths);
    let state = FileStateStore::new(config_root.join("state.toml"));

    let manager = LifecycleManager::new(config, repo_name, repo_root, config_root, &ports, &state);

    operation(&manager)
}

fn lifecycle_context(
    mode: LifecycleMode,
    runner: StdCommandRunner,
    current_dir: &Path,
) -> Result<(PathBuf, String, DinopodConfig)> {
    if mode == LifecycleMode::List {
        let repo_name = current_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repo")
            .to_owned();
        return Ok((
            current_dir.to_path_buf(),
            repo_name,
            load_config(&current_dir.join("dinopod.toml"))?,
        ));
    }

    let preflight = PreflightChecker::new(CommandPreflightProbe::new(runner));
    preflight.require_command(Dependency::Git)?;
    preflight.require_git_repo(current_dir)?;
    let repo_root = resolve_repo_root(runner, current_dir)?;
    let repo_name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo")
        .to_owned();
    let config = load_config(&repo_root.join("dinopod.toml"))?;

    preflight.require_docker_daemon()?;
    preflight.require_docker_compose()?;
    if mode == LifecycleMode::Dev {
        let _ = preflight.check_proxy_port(config.proxy.http_port, &config.proxy.container_name)?;
    }

    Ok((repo_root, repo_name, config))
}

fn resolve_repo_root(runner: StdCommandRunner, current_dir: &Path) -> Result<PathBuf> {
    GitWorktreeManager::new(&runner, StdWorktreeFs).resolve_primary_worktree(current_dir)
}

fn load_config(path: &Path) -> Result<DinopodConfig> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(DinopodConfig::from_toml_str(&contents)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DinopodConfig::default()),
        Err(error) => Err(DinopodError::from(error)),
    }
}

fn config_root() -> PathBuf {
    if let Some(value) = std::env::var_os("DINOPOD_CONFIG_DIR") {
        return PathBuf::from(value);
    }

    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(
            || PathBuf::from(".dinopod"),
            |home| PathBuf::from(home).join(".config").join("dinopod"),
        )
}
