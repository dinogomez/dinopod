//! Configuration loading and default resolution for Dinopod.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Resolved Dinopod configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DinopodConfig {
    /// Application service configuration.
    pub app: AppConfig,
    /// Worktree configuration.
    pub worktree: WorktreeConfig,
    /// Shared proxy configuration.
    pub proxy: ProxyConfig,
}

/// Application service configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    /// Compose service name for the app.
    pub service: String,
    /// Port the app listens on inside its container.
    pub internal_port: u16,
    /// User-owned Compose file.
    pub compose_file: PathBuf,
    /// Branch used as the base for new ticket branches.
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
    app: Option<PartialAppConfig>,
    worktree: Option<PartialWorktreeConfig>,
    proxy: Option<PartialProxyConfig>,
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
}

impl Default for DinopodConfig {
    fn default() -> Self {
        Self {
            app: AppConfig {
                service: "app".to_owned(),
                internal_port: 3000,
                compose_file: PathBuf::from("docker-compose.yml"),
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
    /// Loads configuration from TOML text and fills missing values with defaults.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Toml`] when the TOML text is invalid.
    pub fn from_toml_str(input: &str) -> Result<Self, ConfigError> {
        let partial = toml::from_str::<PartialConfig>(input)?;
        Ok(Self::default().merge_partial(partial))
    }

    fn merge_partial(mut self, partial: PartialConfig) -> Self {
        if let Some(app) = partial.app {
            if let Some(value) = app.service {
                self.app.service = value;
            }
            if let Some(value) = app.internal_port {
                self.app.internal_port = value;
            }
            if let Some(value) = app.compose_file {
                self.app.compose_file = value;
            }
            if let Some(value) = app.default_branch {
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
        self
    }
}

/// Renders a starter `dinopod.toml`.
#[must_use]
pub fn render_starter_config(config: &DinopodConfig) -> String {
    format!(
        concat!(
            "[app]\n",
            "service = \"{service}\"\n",
            "internal_port = {internal_port}\n",
            "compose_file = \"{compose_file}\"\n",
            "default_branch = \"{default_branch}\"\n\n",
            "[worktree]\n",
            "root = \"{worktree_root}\"\n\n",
            "[proxy]\n",
            "host_suffix = \"{host_suffix}\"\n",
            "network = \"{network}\"\n",
            "container_name = \"{container_name}\"\n",
            "http_port = {http_port}\n",
            "image = \"{image}\"\n",
        ),
        service = config.app.service,
        internal_port = config.app.internal_port,
        compose_file = config.app.compose_file.display(),
        default_branch = config.app.default_branch,
        worktree_root = config.worktree.root.display(),
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
