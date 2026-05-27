//! Error types for recoverable Dinopod failures.

use std::path::PathBuf;

use crate::config::ConfigError;
use crate::names::NameError;
use crate::preflight::Dependency;

/// Recoverable Dinopod errors.
#[derive(Debug, thiserror::Error)]
pub enum DinopodError {
    /// A required machine dependency was not found.
    #[error("missing required dependency: {0}")]
    MissingDependency(Dependency),
    /// Docker is installed but the daemon is not available.
    #[error("Docker is not running")]
    DockerDaemonUnavailable,
    /// The current directory is not inside a Git repository.
    #[error("not inside a Git repository")]
    NotInGitRepository,
    /// The proxy HTTP port is occupied by a non-Dinopod process.
    #[error("port {port} is already in use")]
    PortInUse {
        /// The occupied port.
        port: u16,
    },
    /// The expected worktree path exists but is not the expected Git worktree.
    #[error("worktree path already exists and is not managed by Dinopod: {}", path.display())]
    WorktreePathConflict {
        /// Conflicting path.
        path: PathBuf,
    },
    /// A Git command failed.
    #[error("git command failed ({args:?}): {stderr}")]
    GitCommandFailed {
        /// Git arguments.
        args: Vec<String>,
        /// Process exit code when available.
        exit_code: Option<i32>,
        /// Captured standard error.
        stderr: String,
    },
    /// A Docker command failed.
    #[error("docker command failed ({args:?}): {stderr}")]
    DockerCommandFailed {
        /// Docker arguments.
        args: Vec<String>,
        /// Process exit code when available.
        exit_code: Option<i32>,
        /// Captured standard error.
        stderr: String,
    },
    /// Configuration loading failed.
    #[error("{0}")]
    Config(#[from] ConfigError),
    /// A Dinopod config already exists.
    #[error("dinopod config already exists: {}", path.display())]
    ConfigAlreadyExists {
        /// Existing config path.
        path: PathBuf,
    },
    /// Git did not report any worktree root.
    #[error("could not resolve Git worktree root")]
    GitWorktreeRootUnavailable,
    /// The configured Compose file does not exist.
    #[error("compose file does not exist: {}", path.display())]
    ComposeFileMissing {
        /// Missing Compose file path.
        path: PathBuf,
    },
    /// The configured app service is missing from the resolved Compose model.
    #[error("compose service is missing: {service}")]
    ComposeServiceMissing {
        /// Missing service name.
        service: String,
    },
    /// Docker Compose JSON output could not be inspected.
    #[error("failed to inspect compose config JSON: {0}")]
    ComposeConfigInvalid(#[from] serde_json::Error),
    /// Name derivation failed.
    #[error("{0}")]
    Name(#[from] NameError),
    /// Local state could not be decoded.
    #[error("failed to decode state file: {0}")]
    StateDecode(#[from] toml::de::Error),
    /// Local state could not be encoded.
    #[error("failed to encode state file: {0}")]
    StateEncode(#[from] toml::ser::Error),
    /// Environment could not be found in local state.
    #[error("environment is not tracked: {ticket}")]
    EnvironmentNotFound {
        /// Ticket or slug requested by the user.
        ticket: String,
    },
    /// A destructive action requires explicit confirmation.
    #[error("confirmation required before removing environment: {ticket}")]
    ConfirmationRequired {
        /// Ticket or slug requested by the user.
        ticket: String,
    },
    /// Another Dinopod process holds the lifecycle lock.
    #[error("another dinopod command is already running; lock file: {}", path.display())]
    LockUnavailable {
        /// Lock file path.
        path: PathBuf,
    },
    /// A local I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Dinopod result alias.
pub type Result<T> = std::result::Result<T, DinopodError>;
