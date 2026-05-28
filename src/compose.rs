//! Docker Compose validation and Dinopod-owned override generation.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cmd::{path_display, CommandSpec};
use crate::config::{DinopodConfig, RuntimeMode};
use crate::env::PortPlan;
use crate::errors::{DinopodError, Result};
use crate::names::EnvironmentNames;

/// Filesystem checks used by Compose validation.
pub trait ComposeFs {
    /// Returns true when `path` is an existing file.
    fn file_exists(&self, path: &Path) -> bool;
}

/// Production filesystem probe for Compose files.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdComposeFs;

impl ComposeFs for StdComposeFs {
    fn file_exists(&self, path: &Path) -> bool {
        path.is_file()
    }
}

/// Validates local Compose inputs before command execution.
#[derive(Debug)]
pub struct ComposeValidator<F> {
    fs: F,
}

impl<F> ComposeValidator<F>
where
    F: ComposeFs,
{
    /// Creates a Compose validator.
    #[must_use]
    pub fn new(fs: F) -> Self {
        Self { fs }
    }

    /// Requires a Compose file to exist.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::ComposeFileMissing`] when the file is absent.
    pub fn require_compose_file(&self, path: &Path) -> Result<()> {
        if self.fs.file_exists(path) {
            Ok(())
        } else {
            Err(DinopodError::ComposeFileMissing {
                path: path.to_path_buf(),
            })
        }
    }
}

/// User Compose file plus the Dinopod-owned override file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeFiles {
    user_file: PathBuf,
    override_file: PathBuf,
}

impl ComposeFiles {
    /// Creates a Compose file pair.
    #[must_use]
    pub fn new(user_file: impl Into<PathBuf>, override_file: impl Into<PathBuf>) -> Self {
        Self {
            user_file: user_file.into(),
            override_file: override_file.into(),
        }
    }

    /// Returns the user-owned Compose file.
    #[must_use]
    pub fn user_file(&self) -> &Path {
        &self.user_file
    }

    /// Returns the Dinopod-owned Compose override file.
    #[must_use]
    pub fn override_file(&self) -> &Path {
        &self.override_file
    }
}

/// Non-fatal findings discovered from Compose config inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComposeWarning {
    /// A service publishes a fixed host port and can collide across projects.
    FixedHostPort {
        /// Service that publishes the port.
        service: String,
        /// Published host port.
        published: String,
    },
    /// A service uses a fixed container name and can collide across projects.
    FixedContainerName {
        /// Service that sets the container name.
        service: String,
        /// Literal Docker container name.
        container_name: String,
    },
}

impl fmt::Display for ComposeWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FixedHostPort { service, published } => write!(
                formatter,
                "fixed host port published by service `{service}`: {published}"
            ),
            Self::FixedContainerName {
                service,
                container_name,
            } => write!(
                formatter,
                "fixed container name on service `{service}`: {container_name}"
            ),
        }
    }
}

/// Result of inspecting Docker Compose's canonical JSON model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeInspection {
    warnings: Vec<ComposeWarning>,
    /// Whether the app service relies on Compose's implicit `default` network.
    attach_implicit_default_network: bool,
    /// Service names from the resolved Compose model.
    service_names: Vec<String>,
}

impl Default for ComposeInspection {
    fn default() -> Self {
        Self {
            warnings: Vec::new(),
            attach_implicit_default_network: true,
            service_names: Vec::new(),
        }
    }
}

impl ComposeInspection {
    /// Returns non-fatal warnings from the inspection.
    #[must_use]
    pub fn warnings(&self) -> &[ComposeWarning] {
        &self.warnings
    }

    /// Returns whether the Dinopod override should attach the implicit `default` network.
    #[must_use]
    pub fn attach_implicit_default_network(&self) -> bool {
        self.attach_implicit_default_network
    }

    /// Returns service names from the resolved Compose model.
    #[must_use]
    pub fn service_names(&self) -> &[String] {
        &self.service_names
    }

    /// Creates an inspection result for tests and fakes.
    #[must_use]
    pub fn with_service_names(service_names: Vec<String>) -> Self {
        Self {
            service_names,
            ..Self::default()
        }
    }
}

/// Inspects `docker compose config --format json` output for MVP requirements.
///
/// # Errors
///
/// Returns a JSON inspection error when the text is invalid, or
/// [`DinopodError::ComposeServiceMissing`] when `app_service` is absent.
pub fn inspect_compose_config(input: &str, app_service: &str) -> Result<ComposeInspection> {
    inspect_compose_config_for_runtime(input, app_service, RuntimeMode::Container)
}

/// Inspects Compose JSON for the selected runtime mode.
///
/// # Errors
///
/// Returns a JSON inspection error when the text is invalid, or
/// [`DinopodError::ComposeServiceMissing`] when container mode requires a missing app service.
/// Inspects Compose JSON without requiring a specific app service.
///
/// # Errors
///
/// Returns [`DinopodError::ComposeServicesMissing`] when no services are defined.
pub fn inspect_compose_services(input: &str) -> Result<ComposeInspection> {
    let value = serde_json::from_str::<Value>(input)?;
    let Some(services) = value.get("services").and_then(Value::as_object) else {
        return Err(DinopodError::ComposeServicesMissing);
    };
    if services.is_empty() {
        return Err(DinopodError::ComposeServicesMissing);
    }

    let service_names = services.keys().cloned().collect::<Vec<_>>();
    let mut warnings = Vec::new();
    for (service_name, service) in services {
        warnings.extend(fixed_host_port_warnings(service_name, service));
        warnings.extend(fixed_container_name_warnings(service_name, service));
    }

    Ok(ComposeInspection {
        warnings,
        attach_implicit_default_network: true,
        service_names,
    })
}

/// Renders a combined override for pod compose: isolated ports and optional proxy attachment.
#[must_use]
pub fn render_pod_override(
    compose_config: &Value,
    config: &DinopodConfig,
    names: &EnvironmentNames,
    port_plan: &PortPlan,
    service_names: &[String],
    attach_implicit_default_network: bool,
) -> String {
    let app_service = &config.app.service;
    let infra_services: Vec<String> = service_names
        .iter()
        .filter(|name| *name != app_service)
        .cloned()
        .collect();
    let mut body = render_infra_override(compose_config, app_service, port_plan, &infra_services);
    if service_names.iter().any(|name| name == app_service) {
        let network_block = render_override(config, names, attach_implicit_default_network);
        if let Some(network_section) = network_block.split_once("services:\n") {
            body.push('\n');
            body.push_str(network_section.1.trim_start());
        }
    }
    body
}

pub fn inspect_compose_config_for_runtime(
    input: &str,
    app_service: &str,
    runtime: RuntimeMode,
) -> Result<ComposeInspection> {
    let value = serde_json::from_str::<Value>(input)?;
    let Some(services) = value.get("services").and_then(Value::as_object) else {
        return Err(DinopodError::ComposeServicesMissing);
    };

    if services.is_empty() {
        return Err(DinopodError::ComposeServicesMissing);
    }

    let service_names = services.keys().cloned().collect::<Vec<_>>();
    if runtime == RuntimeMode::Container && !services.contains_key(app_service) {
        return Err(DinopodError::ComposeServiceMissing {
            service: app_service.to_owned(),
        });
    }

    let attach_implicit_default_network = services
        .get(app_service)
        .is_none_or(service_uses_implicit_default_network);

    let mut warnings = Vec::new();
    for (service_name, service) in services {
        warnings.extend(fixed_host_port_warnings(service_name, service));
        warnings.extend(fixed_container_name_warnings(service_name, service));
    }

    Ok(ComposeInspection {
        warnings,
        attach_implicit_default_network,
        service_names,
    })
}

/// Returns infra service names, excluding the configured app service when present.
#[must_use]
pub fn infra_service_names(service_names: &[String], app_service: &str) -> Vec<String> {
    service_names
        .iter()
        .filter(|name| **name != app_service)
        .cloned()
        .collect()
}

/// Renders a Dinopod override that publishes deterministic infra host ports.
#[must_use]
pub fn render_infra_override(
    compose_config: &Value,
    app_service: &str,
    port_plan: &PortPlan,
    infra_services: &[String],
) -> String {
    let mut lines = vec![
        "# Generated by Dinopod. Do not edit.".to_owned(),
        "services:".to_owned(),
    ];
    let services = compose_config.get("services").and_then(Value::as_object);

    for service_name in infra_services {
        let Some(host_port) = host_port_for_service(service_name, app_service, port_plan) else {
            continue;
        };
        let container_port = services
            .and_then(|all| all.get(service_name))
            .and_then(default_container_port)
            .unwrap_or(default_service_container_port(service_name));
        lines.push(format!("  {service_name}:"));
        lines.push("    ports: !override".to_owned());
        lines.push(format!("      - \"{host_port}:{container_port}\""));
    }

    let _ = app_service;
    format!("{}\n", lines.join("\n"))
}

/// Validates that infra services do not publish host ports outside the ticket port plan.
///
/// # Errors
///
/// Returns [`DinopodError::InfraHostPortConflict`] when a conflicting host binding remains.
pub fn validate_infra_host_ports(
    compose_config: &Value,
    app_service: &str,
    port_plan: &PortPlan,
) -> Result<()> {
    let allowed = allowed_infra_host_ports(port_plan);
    let Some(services) = compose_config.get("services").and_then(Value::as_object) else {
        return Ok(());
    };

    for (service_name, service) in services {
        if service_name == app_service {
            continue;
        }
        for port in published_host_ports(service) {
            if !allowed.contains(&port) {
                return Err(DinopodError::InfraHostPortConflict {
                    service: service_name.clone(),
                    published: port,
                });
            }
        }
    }

    Ok(())
}

/// Drops infra port warnings for host ports managed by Dinopod's override.
#[must_use]
pub fn filter_managed_port_warnings(
    warnings: Vec<ComposeWarning>,
    port_plan: &PortPlan,
) -> Vec<ComposeWarning> {
    let allowed = allowed_infra_host_ports(port_plan)
        .into_iter()
        .map(|port| port.to_string())
        .collect::<HashSet<_>>();

    warnings
        .into_iter()
        .filter(|warning| match warning {
            ComposeWarning::FixedHostPort { published, .. } => !allowed.contains(published),
            ComposeWarning::FixedContainerName { .. } => true,
        })
        .collect()
}

/// Builds a `docker compose up -d` command for explicit infra services.
#[must_use]
pub fn build_compose_infra_up_command<I, S>(
    project_name: &str,
    files: &ComposeFiles,
    services: I,
) -> CommandSpec
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = vec!["up".to_owned(), "-d".to_owned()];
    args.extend(services.into_iter().map(Into::into));
    build_compose_command(project_name, files, args)
}

fn service_uses_implicit_default_network(service: &Value) -> bool {
    match service.get("networks") {
        None | Some(Value::Null) => true,
        Some(Value::Object(networks)) if networks.is_empty() => true,
        Some(Value::Array(networks)) if networks.is_empty() => true,
        Some(_) => false,
    }
}

/// Renders the Dinopod Compose override that attaches the app to the proxy network.
#[must_use]
pub fn render_override(
    config: &DinopodConfig,
    names: &EnvironmentNames,
    attach_implicit_default_network: bool,
) -> String {
    let default_network = if attach_implicit_default_network {
        "      default: {}\n"
    } else {
        ""
    };

    format!(
        concat!(
            "# Generated by Dinopod. Do not edit.\n",
            "services:\n",
            "  {service}:\n",
            "    networks:\n",
            "{default_network}",
            "      {network}:\n",
            "        aliases:\n",
            "          - {alias}\n",
            "networks:\n",
            "  {network}:\n",
            "    external: true\n",
            "    name: {network}\n",
        ),
        service = config.app.service,
        default_network = default_network,
        network = config.proxy.network,
        alias = names.network_alias.as_str(),
    )
}

/// Builds a `docker compose` command using the user file and Dinopod override.
#[must_use]
pub fn build_compose_command<I, S>(project_name: &str, files: &ComposeFiles, args: I) -> CommandSpec
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut compose_args = vec![
        "compose".to_owned(),
        "-p".to_owned(),
        project_name.to_owned(),
        "-f".to_owned(),
        path_display(files.user_file()),
        "-f".to_owned(),
        path_display(files.override_file()),
    ];
    compose_args.extend(args.into_iter().map(Into::into));
    CommandSpec::new("docker").args(compose_args)
}

fn fixed_host_port_warnings(app_service: &str, service: &Value) -> Vec<ComposeWarning> {
    service
        .get("ports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|port| {
            port.get("published")
                .and_then(published_port)
                .map(|published| ComposeWarning::FixedHostPort {
                    service: app_service.to_owned(),
                    published,
                })
        })
        .collect()
}

fn fixed_container_name_warnings(service_name: &str, service: &Value) -> Vec<ComposeWarning> {
    service
        .get("container_name")
        .and_then(Value::as_str)
        .map(|container_name| ComposeWarning::FixedContainerName {
            service: service_name.to_owned(),
            container_name: container_name.to_owned(),
        })
        .into_iter()
        .collect()
}

fn published_port(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_u64().map(|port| port.to_string()))
}

fn published_host_ports(service: &Value) -> Vec<u16> {
    service
        .get("ports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|port| {
            port.get("published")
                .and_then(published_port)
                .and_then(|published| published.parse().ok())
        })
        .collect()
}

fn allowed_infra_host_ports(port_plan: &PortPlan) -> Vec<u16> {
    [port_plan.postgres_host_port, port_plan.redis_host_port]
        .into_iter()
        .flatten()
        .collect()
}

fn host_port_for_service(
    service_name: &str,
    app_service: &str,
    port_plan: &PortPlan,
) -> Option<u16> {
    if service_name == app_service {
        Some(port_plan.app_host_port)
    } else if service_name == "redis" {
        port_plan.redis_host_port
    } else if service_name == "db" || service_name.starts_with("postgres") {
        port_plan.postgres_host_port
    } else {
        None
    }
}

fn default_container_port(service: &Value) -> Option<u16> {
    service
        .get("ports")
        .and_then(Value::as_array)
        .and_then(|ports| ports.first())
        .and_then(|port| port.get("target"))
        .and_then(|value| value.as_u64().and_then(|port| u16::try_from(port).ok()))
}

fn default_service_container_port(service_name: &str) -> u16 {
    if service_name == "redis" {
        6379
    } else {
        5432
    }
}
