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
    /// `dinopod.toml` is required but missing.
    #[error("dinopod.toml not found at {}; run `dinopod init` first", path.display())]
    ConfigRequired {
        /// Expected config path.
        path: PathBuf,
    },
    /// A configured setup command failed.
    #[error("setup command failed (`{command}`): {stderr}")]
    SetupCommandFailed {
        /// Command string from configuration.
        command: String,
        /// Captured stderr or error detail.
        stderr: String,
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
    /// The resolved Compose model does not define any services.
    #[error("compose file does not define any services")]
    ComposeServicesMissing,
    /// An infra service publishes a host port outside the ticket port plan.
    #[error(
        "infra service `{service}` publishes host port {published}, which conflicts with Dinopod port isolation; remove fixed host ports from docker-compose.override.yml"
    )]
    InfraHostPortConflict {
        /// Infra service name.
        service: String,
        /// Conflicting published host port.
        published: u16,
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
    /// `exec` was invoked without a command after `--`.
    #[error("command required; usage: dinopod <ID> <command...>")]
    ExecCommandRequired,
    /// A destructive action requires explicit confirmation.
    #[error("confirmation required before removing environment: {ticket}; re-run with --yes or confirm at the prompt")]
    ConfirmationRequired {
        /// Ticket or slug requested by the user.
        ticket: String,
    },
    /// Another Dinopod process holds the lifecycle guard.
    #[error("another dinopod command is already running{detail}; guard file: {}", path.display())]
    LockUnavailable {
        /// Guard file path.
        path: PathBuf,
        /// Optional holder pid suffix, e.g. ` (pid 12345)`.
        detail: String,
    },
    /// Local state could not be persisted after containers started.
    #[error("environment {project} is running but state could not be saved; run `dinopod list --reconcile`")]
    StatePersistFailed {
        /// Compose project name.
        project: String,
        /// Underlying persistence error.
        #[source]
        source: Box<DinopodError>,
    },
    /// A local I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Root `package.json` is required for native dev but missing.
    #[error("package.json is missing at the repository root")]
    PackageJsonMissing,
    /// Root `package.json` could not be parsed.
    #[error("failed to parse package.json: {0}")]
    PackageJsonInvalid(serde_json::Error),
    /// The requested npm/pnpm dev script is missing.
    #[error("dev script not found; available scripts: {available}")]
    DevScriptMissing {
        /// Comma-separated script names from `package.json`.
        available: String,
    },
    /// Both native and container signals are present without an explicit runtime.
    #[error(
        "project has both a root package.json and a compose app service; set runtime = \"native\" or runtime = \"container\" in dinopod.toml"
    )]
    RuntimeModeAmbiguous,
    /// Neither native nor container project signals were detected.
    #[error(
        "could not detect project type; add a root package.json for native dev or an app compose service for container mode (see `dinopod init`)"
    )]
    ProjectTypeUnknown,
    /// Env file symlink copy was rejected for safety.
    #[error("refusing to copy symlink env file: {}", path.display())]
    EnvSymlinkRejected {
        /// Rejected env file path.
        path: PathBuf,
    },
    /// No free host port remained in a required allocation range.
    #[error("no free host port available for {service} in range {range}")]
    PortRangeExhausted {
        /// Infra or app service name.
        service: String,
        /// Exhausted port range.
        range: String,
    },
    /// Native dev process spawn failed.
    #[error("failed to spawn native dev process: {stderr}")]
    DevProcessSpawnFailed {
        /// Captured spawn stderr.
        stderr: String,
    },
    /// Native dev PID file contained invalid data.
    #[error("invalid dev pid file contents: {contents}")]
    DevProcessPidInvalid {
        /// Raw PID file contents.
        contents: String,
    },
    /// Native dev process exited with a non-zero status in foreground mode.
    #[error("native dev process exited with status {code:?}")]
    DevProcessExited {
        /// Process exit code when available.
        code: Option<i32>,
    },
}

/// Dinopod result alias.
pub type Result<T> = std::result::Result<T, DinopodError>;
