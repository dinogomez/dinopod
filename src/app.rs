//! Application context for Dinopod CLI commands.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cmd::StdCommandRunner;
use crate::config::DinopodConfig;
use crate::errors::{DinopodError, Result};
use crate::git::{GitWorktreeManager, StdWorktreeFs};
use crate::lifecycle::LifecycleManager;
use crate::lock::MutationGuard;
use crate::preflight::{CommandPreflightProbe, Dependency, PreflightChecker};
use crate::proxy::ProxyPaths;
use crate::runtime::CommandLifecyclePorts;
use crate::state::FileStateStore;

/// Shared CLI runtime context for lifecycle commands.
pub struct AppContext {
    config: DinopodConfig,
    repo_name: String,
    repo_root: PathBuf,
    config_root: PathBuf,
    _guard: MutationGuard,
    ports: CommandLifecyclePorts<StdCommandRunner>,
    state: FileStateStore,
}

impl AppContext {
    /// Builds an application context for lifecycle commands that mutate environments.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when preflight checks or guard acquisition fails.
    pub fn for_mutating(current_dir: &Path, check_proxy_port: bool) -> Result<Self> {
        Self::build(current_dir, PreflightProfile::Mutating { check_proxy_port })
    }

    /// Builds an application context for `dev`.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when preflight checks or guard acquisition fails.
    pub fn for_dev(current_dir: &Path) -> Result<Self> {
        Self::for_mutating(current_dir, true)
    }

    /// Builds an application context for read-only listing.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when configuration cannot be loaded or the guard
    /// is already held.
    pub fn for_list(current_dir: &Path) -> Result<Self> {
        Self::build(current_dir, PreflightProfile::List)
    }

    /// Returns a lifecycle manager bound to this context.
    #[must_use]
    pub fn lifecycle_manager(
        &self,
    ) -> LifecycleManager<'_, CommandLifecyclePorts<StdCommandRunner>, FileStateStore> {
        LifecycleManager::new(
            self.config.clone(),
            self.repo_name.clone(),
            self.repo_root.clone(),
            self.config_root.clone(),
            &self.ports,
            &self.state,
        )
    }

    fn build(current_dir: &Path, profile: PreflightProfile) -> Result<Self> {
        let runner = StdCommandRunner;
        let (repo_root, repo_name, config) = match profile {
            PreflightProfile::List => (
                current_dir.to_path_buf(),
                directory_name(current_dir),
                load_config(&current_dir.join("dinopod.toml"))?,
            ),
            PreflightProfile::Mutating { check_proxy_port } => {
                let preflight = PreflightChecker::new(CommandPreflightProbe::new(runner));
                preflight.require_command(Dependency::Git)?;
                preflight.require_git_repo(current_dir)?;
                let repo_root = GitWorktreeManager::new(&StdCommandRunner, StdWorktreeFs)
                    .resolve_primary_worktree(current_dir)?;
                let repo_name = directory_name(&repo_root);
                let config = load_config(&repo_root.join("dinopod.toml"))?;
                preflight.require_docker_daemon()?;
                preflight.require_docker_compose()?;
                if check_proxy_port {
                    let _ = preflight
                        .check_proxy_port(config.proxy.http_port, &config.proxy.container_name)?;
                }
                (repo_root, repo_name, config)
            }
        };

        let config_root = config_root();
        let guard_path = config_root.join("dinopod.lock");
        let Some(guard) = MutationGuard::try_acquire(&guard_path)? else {
            return Err(DinopodError::LockUnavailable { path: guard_path });
        };
        let proxy_paths = ProxyPaths::new(&config_root);
        let ports = CommandLifecyclePorts::new(StdCommandRunner, config.clone(), proxy_paths);
        let state = FileStateStore::new(config_root.join("state.toml"));

        Ok(Self {
            config,
            repo_name,
            repo_root,
            config_root,
            _guard: guard,
            ports,
            state,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreflightProfile {
    List,
    Mutating { check_proxy_port: bool },
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

fn directory_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo")
        .to_owned()
}
