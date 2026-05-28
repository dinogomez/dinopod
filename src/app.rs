//! Application context for Dinopod CLI commands.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cmd::StdCommandRunner;
use crate::config::DinopodConfig;
use crate::errors::{DinopodError, Result};
use crate::git::{GitWorktreeManager, StdWorktreeFs};
use crate::lifecycle::LifecycleManager;
use crate::lock::{lock_unavailable_error, MutationGuard};
use crate::preflight::{CommandPreflightProbe, Dependency, PreflightChecker};
use crate::proxy::ProxyPaths;
use crate::runtime::CommandLifecyclePorts;
use crate::state::FileStateStore;

/// Shared CLI runtime context for lifecycle commands.
pub struct AppContext {
    config: DinopodConfig,
    repo_name: String,
    repo_root: PathBuf,
    env_source_root: PathBuf,
    config_root: PathBuf,
    _guard: Option<MutationGuard>,
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
        Self::build(
            current_dir,
            PreflightProfile::Mutating { check_proxy_port },
            GuardPolicy::Required,
        )
    }

    /// Builds an application context for `dinopod new`.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when `dinopod.toml` is missing or preflight fails.
    pub fn for_new(current_dir: &Path) -> Result<Self> {
        Self::build(
            current_dir,
            PreflightProfile::New {
                check_proxy_port: true,
            },
            GuardPolicy::Required,
        )
    }

    /// Builds an application context for `dev`.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when preflight checks or guard acquisition fails.
    pub fn for_dev(current_dir: &Path) -> Result<Self> {
        Self::for_mutating(current_dir, true)
    }

    /// Builds a read-only context for `dinopod list` without acquiring the mutation guard.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when configuration cannot be loaded.
    pub fn for_list(current_dir: &Path) -> Result<Self> {
        Self::build(current_dir, PreflightProfile::List, GuardPolicy::Skip)
    }

    /// Builds a context for `dinopod list --reconcile`, which mutates cached state.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when configuration cannot be loaded or the guard
    /// is already held.
    pub fn for_list_reconcile(current_dir: &Path) -> Result<Self> {
        Self::build(current_dir, PreflightProfile::List, GuardPolicy::Required)
    }

    /// Builds a read-only context for `dinopod <id> <command>` passthrough.
    ///
    /// Does not acquire the mutation guard so long-running child processes do not block other
    /// Dinopod invocations.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when configuration cannot be loaded.
    pub fn for_run(current_dir: &Path) -> Result<Self> {
        Self::build(current_dir, PreflightProfile::Run, GuardPolicy::Skip)
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
            self.env_source_root.clone(),
            self.config_root.clone(),
            &self.ports,
            &self.state,
        )
    }

    /// Returns the primary Git repository root.
    #[must_use]
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn build(
        current_dir: &Path,
        profile: PreflightProfile,
        guard_policy: GuardPolicy,
    ) -> Result<Self> {
        let runner = StdCommandRunner;
        let (repo_root, repo_name, config) = match profile {
            PreflightProfile::List => (
                current_dir.to_path_buf(),
                directory_name(current_dir),
                load_config(&current_dir.join("dinopod.toml"))?,
            ),
            PreflightProfile::Run => {
                let preflight = PreflightChecker::new(CommandPreflightProbe::new(runner));
                preflight.require_git_repo(current_dir)?;
                let repo_root = GitWorktreeManager::new(&StdCommandRunner, StdWorktreeFs)
                    .resolve_primary_worktree(current_dir)?;
                let repo_name = directory_name(&repo_root);
                let config = load_config(&repo_root.join("dinopod.toml"))?;
                (repo_root, repo_name, config)
            }
            PreflightProfile::New { check_proxy_port }
            | PreflightProfile::Mutating { check_proxy_port } => {
                let preflight = PreflightChecker::new(CommandPreflightProbe::new(runner));
                preflight.require_command(Dependency::Git)?;
                preflight.require_git_repo(current_dir)?;
                let repo_root = GitWorktreeManager::new(&StdCommandRunner, StdWorktreeFs)
                    .resolve_primary_worktree(current_dir)?;
                let repo_name = directory_name(&repo_root);
                let config_path = repo_root.join("dinopod.toml");
                let config = match profile {
                    PreflightProfile::New { .. } => load_config_required(&config_path)?,
                    _ => load_config(&config_path)?,
                };
                preflight.require_docker_daemon()?;
                preflight.require_docker_compose()?;
                if check_proxy_port && std::env::var_os("DINOPOD_FAKE_LOG").is_none() {
                    let _ = preflight
                        .check_proxy_port(config.proxy.http_port, &config.proxy.container_name)?;
                }
                (repo_root, repo_name, config)
            }
        };

        let config_root = config_root();
        let guard = match guard_policy {
            GuardPolicy::Skip => None,
            GuardPolicy::Required => {
                let guard_path = config_root.join("dinopod.lock");
                match MutationGuard::try_acquire(&guard_path)? {
                    Some(guard) => Some(guard),
                    None => return Err(lock_unavailable_error(guard_path)),
                }
            }
        };
        let env_source_root = repo_root.clone();
        let proxy_paths = ProxyPaths::new(&config_root);
        let ports = CommandLifecyclePorts::new(StdCommandRunner, config.clone(), proxy_paths);
        let state = FileStateStore::new(config_root.join("state.toml"));

        Ok(Self {
            config,
            repo_name,
            repo_root,
            env_source_root,
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
    Run,
    New { check_proxy_port: bool },
    Mutating { check_proxy_port: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuardPolicy {
    Skip,
    Required,
}

fn load_config(path: &Path) -> Result<DinopodConfig> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(DinopodConfig::from_toml_str(&contents)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DinopodConfig::default()),
        Err(error) => Err(DinopodError::from(error)),
    }
}

fn load_config_required(path: &Path) -> Result<DinopodConfig> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(DinopodConfig::from_toml_str(&contents)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(DinopodError::ConfigRequired {
                path: path.to_path_buf(),
            })
        }
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
