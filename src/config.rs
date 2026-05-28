//! Configuration loading and default resolution for Dinopod.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum number of setup commands allowed in configuration.
pub const MAX_SETUP_COMMANDS: usize = 32;

/// Default env filename substrings to skip when copying into worktrees.
pub const DEFAULT_ENV_SKIP_PATTERNS: &[&str] = &[".production", ".staging", ".test"];

/// Runtime mode for Dinopod environments (legacy; not exposed in v1 wizard).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeMode {
    /// Run the app on the host and Compose for infra only.
    Native,
    /// Run the app in Docker Compose (MVP behavior).
    Container,
}

/// Resolved Dinopod configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DinopodConfig {
    /// Explicit runtime override from configuration (legacy).
    pub runtime: Option<RuntimeMode>,
    /// Native runtime configuration (legacy).
    pub native: NativeConfig,
    /// User Compose file path.
    pub compose: ComposeConfig,
    /// Git defaults for new worktrees.
    pub git: GitConfig,
    /// Project settings.
    pub settings: SettingsConfig,
    /// Commands run after compose on `dinopod new` (created worktrees only).
    pub setup: SetupConfig,
    /// Application service configuration.
    pub app: AppConfig,
    /// Worktree configuration.
    pub worktree: WorktreeConfig,
    /// Shared proxy configuration.
    pub proxy: ProxyConfig,
}

/// Compose file configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeConfig {
    /// User-owned Compose file relative to repo root.
    pub file: PathBuf,
}

/// Git configuration for worktree branches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitConfig {
    /// Branch used as the base for new ticket branches.
    pub default_branch: String,
}

/// Project-wide Dinopod settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsConfig {
    /// Copy dotenv files into new worktrees when created.
    pub copy_env: bool,
    /// Skip env files whose names contain any of these substrings.
    pub env_skip_patterns: Vec<String>,
}

/// Setup commands for `dinopod new`.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct SetupConfig {
    /// Shell commands to run in the worktree after compose is up (created pods only).
    pub commands: Vec<String>,
}

/// Native runtime configuration.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct NativeConfig {
    /// npm/pnpm script override for native dev.
    pub dev_script: Option<String>,
    /// Default app listen port before per-ticket allocation.
    pub app_port: Option<u16>,
}

/// Application service configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    /// Compose service name for the app.
    pub service: String,
    /// Port the app listens on inside its container.
    pub internal_port: u16,
    /// User-owned Compose file (legacy; prefer `[compose].file`).
    pub compose_file: PathBuf,
    /// Branch used as the base for new ticket branches (legacy; prefer `[git]`).
    pub default_branch: String,
}

/// Worktree configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeConfig {
    /// Root directory where Dinopod creates worktrees.
    pub root: PathBuf,
}

/// Shared proxy configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyConfig {
    /// Host suffix for generated environment hostnames.
    pub host_suffix: String,
    /// Shared Docker network used by Traefik and app containers.
    pub network: String,
    /// Shared Traefik container name.
    pub container_name: String,
    /// Host HTTP port exposed by the proxy.
    pub http_port: u16,
    /// Traefik image reference.
    pub image: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialConfig {
    runtime: Option<String>,
    native: Option<PartialNativeConfig>,
    compose: Option<PartialComposeConfig>,
    git: Option<PartialGitConfig>,
    settings: Option<PartialSettingsConfig>,
    setup: Option<PartialSetupConfig>,
    app: Option<PartialAppConfig>,
    worktree: Option<PartialWorktreeConfig>,
    proxy: Option<PartialProxyConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialComposeConfig {
    file: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialGitConfig {
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialSettingsConfig {
    copy_env: Option<bool>,
    env_skip_patterns: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialSetupConfig {
    commands: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialNativeConfig {
    dev_script: Option<String>,
    app_port: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialAppConfig {
    service: Option<String>,
    internal_port: Option<u16>,
    compose_file: Option<PathBuf>,
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialWorktreeConfig {
    root: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialProxyConfig {
    host_suffix: Option<String>,
    network: Option<String>,
    container_name: Option<String>,
    http_port: Option<u16>,
    image: Option<String>,
}

/// Configuration loading errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The TOML configuration could not be parsed.
    #[error("failed to parse dinopod.toml: {0}")]
    Toml(#[from] toml::de::Error),
    /// The configured runtime mode is invalid.
    #[error("invalid runtime mode in dinopod.toml: {0}")]
    InvalidRuntime(String),
    /// A setup command is not allowed.
    #[error("{0}")]
    InvalidSetupCommand(String),
}

impl RuntimeMode {
    fn parse_config_value(value: &str) -> Result<Self, ConfigError> {
        match value {
            "native" => Ok(Self::Native),
            "container" => Ok(Self::Container),
            other => Err(ConfigError::InvalidRuntime(other.to_owned())),
        }
    }
}

impl Default for ComposeConfig {
    fn default() -> Self {
        Self {
            file: PathBuf::from("docker-compose.yml"),
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            default_branch: "main".to_owned(),
        }
    }
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            copy_env: true,
            env_skip_patterns: DEFAULT_ENV_SKIP_PATTERNS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }
    }
}

impl Default for DinopodConfig {
    fn default() -> Self {
        let compose_file = PathBuf::from("docker-compose.yml");
        Self {
            runtime: None,
            native: NativeConfig::default(),
            compose: ComposeConfig {
                file: compose_file.clone(),
            },
            git: GitConfig::default(),
            settings: SettingsConfig::default(),
            setup: SetupConfig::default(),
            app: AppConfig {
                service: "app".to_owned(),
                internal_port: 3000,
                compose_file,
                default_branch: "main".to_owned(),
            },
            worktree: WorktreeConfig {
                root: PathBuf::from("../.dinopod-worktrees"),
            },
            proxy: ProxyConfig {
                host_suffix: "localhost".to_owned(),
                network: "dinopod-proxy".to_owned(),
                container_name: "dinopod-traefik".to_owned(),
                http_port: 80,
                image: "traefik:v3.6".to_owned(),
            },
        }
    }
}

impl DinopodConfig {
    /// Returns the configured Compose file path.
    #[must_use]
    pub fn compose_file(&self) -> &Path {
        &self.compose.file
    }

    /// Returns the default branch for new ticket worktrees.
    #[must_use]
    pub fn default_branch(&self) -> &str {
        &self.git.default_branch
    }

    /// Loads configuration from TOML text and fills missing values with defaults.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the TOML text is invalid or setup commands are disallowed.
    pub fn from_toml_str(input: &str) -> Result<Self, ConfigError> {
        let partial = toml::from_str::<PartialConfig>(input)?;
        let config = Self::default().merge_partial(partial)?;
        config.validate_setup_commands()?;
        Ok(config)
    }

    fn merge_partial(mut self, partial: PartialConfig) -> Result<Self, ConfigError> {
        if let Some(runtime) = partial.runtime {
            self.runtime = Some(RuntimeMode::parse_config_value(&runtime)?);
        }
        if let Some(native) = partial.native {
            if let Some(value) = native.dev_script {
                self.native.dev_script = Some(value);
            }
            if let Some(value) = native.app_port {
                self.native.app_port = Some(value);
            }
        }
        if let Some(compose) = partial.compose {
            if let Some(value) = compose.file {
                self.compose.file.clone_from(&value);
                self.app.compose_file.clone_from(&value);
            }
        }
        if let Some(git) = partial.git {
            if let Some(value) = git.default_branch {
                self.git.default_branch.clone_from(&value);
                self.app.default_branch = value;
            }
        }
        if let Some(settings) = partial.settings {
            if let Some(value) = settings.copy_env {
                self.settings.copy_env = value;
            }
            if let Some(value) = settings.env_skip_patterns {
                self.settings.env_skip_patterns = value;
            }
        }
        if let Some(setup) = partial.setup {
            if let Some(value) = setup.commands {
                self.setup.commands = value;
            }
        }
        if let Some(app) = partial.app {
            if let Some(value) = app.service {
                self.app.service = value;
            }
            if let Some(value) = app.internal_port {
                self.app.internal_port = value;
            }
            if let Some(value) = app.compose_file {
                self.compose.file.clone_from(&value);
                self.app.compose_file.clone_from(&value);
            }
            if let Some(value) = app.default_branch {
                self.git.default_branch.clone_from(&value);
                self.app.default_branch = value;
            }
        }
        if let Some(worktree) = partial.worktree {
            if let Some(value) = worktree.root {
                self.worktree.root = value;
            }
        }
        if let Some(proxy) = partial.proxy {
            if let Some(value) = proxy.host_suffix {
                self.proxy.host_suffix = value;
            }
            if let Some(value) = proxy.network {
                self.proxy.network = value;
            }
            if let Some(value) = proxy.container_name {
                self.proxy.container_name = value;
            }
            if let Some(value) = proxy.http_port {
                self.proxy.http_port = value;
            }
            if let Some(value) = proxy.image {
                self.proxy.image = value;
            }
        }
        Ok(self)
    }

    fn validate_setup_commands(&self) -> Result<(), ConfigError> {
        if self.setup.commands.len() > MAX_SETUP_COMMANDS {
            return Err(ConfigError::InvalidSetupCommand(format!(
                "at most {MAX_SETUP_COMMANDS} setup commands are allowed"
            )));
        }
        for command in &self.setup.commands {
            validate_setup_command(command)?;
        }
        Ok(())
    }
}

/// Returns true when `name` should be copied as a dotenv file.
#[must_use]
pub fn should_copy_env_file(name: &str, skip_patterns: &[String]) -> bool {
    if !is_dotenv_file_name(name) {
        return false;
    }
    !skip_patterns.iter().any(|pattern| name.contains(pattern))
}

/// Returns true when `name` is an env file considered for copy.
#[must_use]
pub fn is_dotenv_file_name(name: &str) -> bool {
    name.contains(".env")
}

/// Rejects setup commands that run Docker Compose directly.
///
/// # Errors
///
/// Returns [`ConfigError::InvalidSetupCommand`] when the command invokes compose.
pub fn validate_setup_command(command: &str) -> Result<(), ConfigError> {
    let lower = command.to_ascii_lowercase();
    if lower.contains("docker compose") || lower.contains("docker-compose") {
        return Err(ConfigError::InvalidSetupCommand(
            "setup commands must not run docker compose; Dinopod starts compose during `dinopod new`"
                .to_owned(),
        ));
    }
    Ok(())
}

/// Renders a starter `dinopod.toml` from resolved configuration.
#[must_use]
pub fn render_starter_config(config: &DinopodConfig) -> String {
    let skip_patterns = config
        .settings
        .env_skip_patterns
        .iter()
        .map(|pattern| format!("\"{pattern}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let setup_block = if config.setup.commands.is_empty() {
        "[setup]\ncommands = []\n\n".to_owned()
    } else {
        let commands = config
            .setup
            .commands
            .iter()
            .map(|command| format!("  \"{command}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        format!("[setup]\ncommands = [\n{commands},\n]\n\n")
    };

    format!(
        concat!(
            "[compose]\n",
            "file = \"{compose_file}\"\n\n",
            "[settings]\n",
            "copy_env = {copy_env}\n",
            "env_skip_patterns = [{skip_patterns}]\n\n",
            "[worktree]\n",
            "root = \"{worktree_root}\"\n\n",
            "[git]\n",
            "default_branch = \"{default_branch}\"\n\n",
            "{setup_block}",
            "[app]\n",
            "service = \"{service}\"\n",
            "internal_port = {internal_port}\n\n",
            "[proxy]\n",
            "host_suffix = \"{host_suffix}\"\n",
            "network = \"{network}\"\n",
            "container_name = \"{container_name}\"\n",
            "http_port = {http_port}\n",
            "image = \"{image}\"\n",
        ),
        compose_file = config.compose.file.display(),
        copy_env = config.settings.copy_env,
        skip_patterns = skip_patterns,
        worktree_root = config.worktree.root.display(),
        default_branch = config.git.default_branch,
        setup_block = setup_block,
        service = config.app.service,
        internal_port = config.app.internal_port,
        host_suffix = config.proxy.host_suffix,
        network = config.proxy.network,
        container_name = config.proxy.container_name,
        http_port = config.proxy.http_port,
        image = config.proxy.image,
    )
}

impl AsRef<Path> for AppConfig {
    fn as_ref(&self) -> &Path {
        &self.compose_file
    }
}
