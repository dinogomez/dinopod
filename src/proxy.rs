//! Shared Traefik proxy configuration and lifecycle commands.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cmd::{docker_command_failed, path_display, CommandOutput, CommandRunner, CommandSpec};
use crate::config::DinopodConfig;
use crate::errors::Result;

/// Docker inspect format used to compare a running proxy with Dinopod config.
pub const PROXY_INSPECT_FORMAT: &str = concat!(
    "{{.State.Running}}\t{{.Config.Image}}\t",
    "{{json .HostConfig.PortBindings}}\t{{json .Mounts}}"
);

/// Filesystem paths for Dinopod-managed proxy assets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyPaths {
    root: PathBuf,
    compose_file: PathBuf,
    dynamic_config_dir: PathBuf,
}

impl ProxyPaths {
    /// Creates proxy paths under `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let proxy_root = root.join("proxy");
        Self {
            root,
            compose_file: proxy_root.join("compose.yaml"),
            dynamic_config_dir: proxy_root.join("dynamic"),
        }
    }

    /// Returns the Dinopod config root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the generated proxy Compose file path.
    #[must_use]
    pub fn compose_file(&self) -> &Path {
        &self.compose_file
    }

    /// Returns the Dinopod dynamic config directory.
    #[must_use]
    pub fn dynamic_config_dir(&self) -> &Path {
        &self.dynamic_config_dir
    }
}

/// Expected runtime shape for the shared Traefik proxy container.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyRuntimeSpec {
    /// Traefik image reference.
    pub image: String,
    /// Host HTTP port published by the proxy.
    pub http_port: u16,
    /// Host directory mounted into Traefik's dynamic config path.
    pub dynamic_config_dir: PathBuf,
}

impl ProxyRuntimeSpec {
    /// Builds the expected proxy runtime from config and generated paths.
    #[must_use]
    pub fn from_config(config: &DinopodConfig, paths: &ProxyPaths) -> Self {
        Self {
            image: config.proxy.image.clone(),
            http_port: config.proxy.http_port,
            dynamic_config_dir: paths.dynamic_config_dir().to_path_buf(),
        }
    }
}

/// Classifies a running proxy container against Dinopod's expected runtime.
#[must_use]
pub fn classify_proxy_container(inspect_stdout: &str, expected: &ProxyRuntimeSpec) -> ProxyStatus {
    let mut fields = inspect_stdout.split('\t');
    let Some(running) = fields.next() else {
        return ProxyStatus::Stopped;
    };
    let Some(image) = fields.next() else {
        return ProxyStatus::Stopped;
    };
    let port_bindings = fields.next().unwrap_or("null");
    let mounts = fields.next().unwrap_or("null");

    if running != "true" {
        return ProxyStatus::Stopped;
    }
    if image != expected.image {
        return ProxyStatus::NeedsRepair;
    }
    if !host_port_matches(port_bindings, expected.http_port) {
        return ProxyStatus::NeedsRepair;
    }
    if !dynamic_mount_matches(mounts, &expected.dynamic_config_dir) {
        return ProxyStatus::NeedsRepair;
    }

    ProxyStatus::Healthy
}

fn host_port_matches(bindings_json: &str, port: u16) -> bool {
    let Ok(bindings) = serde_json::from_str::<Value>(bindings_json) else {
        return false;
    };
    let key = format!("{port}/tcp");
    bindings
        .get(&key)
        .and_then(Value::as_array)
        .and_then(|bindings| bindings.first())
        .and_then(|binding| binding.get("HostPort"))
        .and_then(Value::as_str)
        .is_some_and(|host_port| host_port == port.to_string())
}

fn dynamic_mount_matches(mounts_json: &str, expected_dir: &Path) -> bool {
    let Ok(mounts) = serde_json::from_str::<Value>(mounts_json) else {
        return false;
    };
    let Some(mounts) = mounts.as_array() else {
        return false;
    };

    mounts.iter().any(|mount| {
        mount.get("Destination").and_then(Value::as_str) == Some("/etc/traefik/dynamic")
            && mount
                .get("Source")
                .and_then(Value::as_str)
                .is_some_and(|source| source == expected_dir.to_string_lossy())
    })
}

/// Observed state of the shared proxy container.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyStatus {
    /// Proxy is healthy and can be reused.
    Healthy,
    /// Proxy is not running.
    Stopped,
    /// Proxy is running with the wrong image or generated config.
    NeedsRepair,
}

/// Action taken by proxy orchestration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyAction {
    /// Existing proxy was reused.
    Reused,
    /// Proxy startup commands were issued.
    Started,
    /// Proxy repair commands were issued.
    Repaired,
}

/// Coordinates shared proxy lifecycle commands through Docker.
#[derive(Debug)]
pub struct ProxyManager<'a, R> {
    runner: &'a R,
}

impl<'a, R> ProxyManager<'a, R>
where
    R: CommandRunner,
{
    /// Creates a proxy manager.
    #[must_use]
    pub fn new(runner: &'a R) -> Self {
        Self { runner }
    }

    /// Ensures the shared proxy is healthy for the provided status.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::DockerCommandFailed`] when Docker rejects a command,
    /// or an I/O error when command execution fails.
    pub fn ensure_proxy(
        &self,
        config: &DinopodConfig,
        paths: &ProxyPaths,
        status: ProxyStatus,
    ) -> Result<ProxyAction> {
        match status {
            ProxyStatus::Healthy => Ok(ProxyAction::Reused),
            ProxyStatus::Stopped => {
                self.start_proxy(config, paths)?;
                Ok(ProxyAction::Started)
            }
            ProxyStatus::NeedsRepair => {
                self.run_docker([
                    "rm".to_owned(),
                    "-f".to_owned(),
                    config.proxy.container_name.clone(),
                ])?;
                self.start_proxy(config, paths)?;
                Ok(ProxyAction::Repaired)
            }
        }
    }

    fn start_proxy(&self, config: &DinopodConfig, paths: &ProxyPaths) -> Result<()> {
        self.ensure_network(&config.proxy.network)?;
        self.run_docker([
            "compose".to_owned(),
            "-p".to_owned(),
            config.proxy.network.clone(),
            "-f".to_owned(),
            path_display(paths.compose_file()),
            "up".to_owned(),
            "-d".to_owned(),
        ])
    }

    fn ensure_network(&self, network: &str) -> Result<()> {
        let inspect_output = self.run_docker_allow_failure(vec![
            "network".to_owned(),
            "inspect".to_owned(),
            network.to_owned(),
        ])?;

        if inspect_output.success() {
            return Ok(());
        }

        self.run_docker([
            "network".to_owned(),
            "create".to_owned(),
            network.to_owned(),
        ])
    }

    fn run_docker<I, S>(&self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let output = self.run_docker_allow_failure(args.clone())?;
        if output.success() {
            Ok(())
        } else {
            Err(docker_command_failed(args, &output))
        }
    }

    fn run_docker_allow_failure(&self, args: Vec<String>) -> io::Result<CommandOutput> {
        self.runner.run(&CommandSpec::new("docker").args(args))
    }
}

/// Renders the Docker Compose file for the shared Traefik proxy.
#[must_use]
pub fn render_proxy_compose(config: &DinopodConfig, paths: &ProxyPaths) -> String {
    format!(
        concat!(
            "# Generated by Dinopod. Do not edit.\n",
            "services:\n",
            "  traefik:\n",
            "    image: {image}\n",
            "    container_name: {container}\n",
            "    command:\n",
            "      - --providers.file.directory=/etc/traefik/dynamic\n",
            "      - --providers.file.watch=true\n",
            "      - --entrypoints.web.address=:{port}\n",
            "    ports:\n",
            "      - \"{port}:{port}\"\n",
            "    volumes:\n",
            "      - {dynamic_dir}:/etc/traefik/dynamic:ro\n",
            "    networks:\n",
            "      - {network}\n",
            "networks:\n",
            "  {network}:\n",
            "    external: true\n",
            "    name: {network}\n",
        ),
        image = config.proxy.image,
        container = config.proxy.container_name,
        port = config.proxy.http_port,
        dynamic_dir = paths.dynamic_config_dir().display(),
        network = config.proxy.network,
    )
}
