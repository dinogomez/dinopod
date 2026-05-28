//! Node project detection and native vs container runtime resolution.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::{DinopodConfig, RuntimeMode};
use crate::errors::{DinopodError, Result};

/// Detected JavaScript package manager.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageManager {
    /// pnpm (lockfile: `pnpm-lock.yaml`).
    Pnpm,
    /// npm (lockfile: `package-lock.json`).
    Npm,
}

/// Resolved project profile consumed by lifecycle orchestration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectProfile {
    /// Selected runtime mode for the environment.
    pub runtime: RuntimeMode,
    /// Package manager when running in native mode.
    pub package_manager: Option<PackageManager>,
    /// npm/pnpm script name to run for native dev.
    pub dev_script: Option<String>,
    /// Whether the project depends on Next.js.
    pub is_nextjs: bool,
    /// Framework default listen port before per-ticket allocation.
    pub default_app_port: u16,
}

/// Filesystem access used by project detection.
pub trait DetectFs {
    /// Returns true when `path` is an existing file.
    fn file_exists(&self, path: &Path) -> bool;

    /// Reads a UTF-8 file from disk.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file cannot be read.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
}

/// Production filesystem probe for project detection.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdDetectFs;

impl DetectFs for StdDetectFs {
    fn file_exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// Returns the conventional root `package.json` path for a repository.
#[must_use]
pub fn package_json_path(repo_root: &Path) -> PathBuf {
    repo_root.join("package.json")
}

/// Parses `package.json` contents.
///
/// # Errors
///
/// Returns [`DinopodError::PackageJsonInvalid`] when JSON parsing fails.
pub fn parse_package_json(contents: &str) -> Result<Value> {
    serde_json::from_str(contents).map_err(DinopodError::PackageJsonInvalid)
}

/// Returns true when `next` appears in runtime dependencies.
#[must_use]
pub fn is_nextjs_project(package_json: &Value) -> bool {
    has_dependency(package_json, "dependencies") || has_dependency(package_json, "devDependencies")
}

fn has_dependency(package_json: &Value, field: &str) -> bool {
    package_json
        .get(field)
        .and_then(Value::as_object)
        .is_some_and(|deps| deps.contains_key("next"))
}

/// Detects the package manager from lockfiles at the repository root.
#[must_use]
pub fn detect_package_manager(repo_root: &Path, fs: &impl DetectFs) -> Option<PackageManager> {
    if fs.file_exists(&repo_root.join("pnpm-lock.yaml")) {
        return Some(PackageManager::Pnpm);
    }
    if fs.file_exists(&repo_root.join("package-lock.json")) {
        return Some(PackageManager::Npm);
    }

    let package_json_path = package_json_path(repo_root);
    if fs.file_exists(&package_json_path) {
        if let Ok(contents) = fs.read_to_string(&package_json_path) {
            if let Ok(package_json) = parse_package_json(&contents) {
                return package_manager_from_package_json(&package_json);
            }
        }
    }

    None
}

fn package_manager_from_package_json(package_json: &Value) -> Option<PackageManager> {
    let manager = package_json.get("packageManager")?.as_str()?;
    if manager.starts_with("pnpm") {
        Some(PackageManager::Pnpm)
    } else if manager.starts_with("npm") {
        Some(PackageManager::Npm)
    } else {
        None
    }
}

/// Returns service names from a Docker Compose config JSON document.
#[must_use]
pub fn service_names_from_compose_config(config: &Value) -> Vec<String> {
    config
        .get("services")
        .and_then(Value::as_object)
        .map_or_else(Vec::new, |services| {
            services.keys().cloned().collect::<Vec<_>>()
        })
}

/// Returns true when the configured app service exists in Compose.
#[must_use]
pub fn compose_has_app_service(service_names: &[String], app_service_name: &str) -> bool {
    service_names.iter().any(|name| name == app_service_name)
}

/// Resolves runtime mode using config overrides and auto-detection signals.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when the project type is ambiguous or unknown.
pub fn resolve_runtime_mode(
    config: &DinopodConfig,
    has_package_json: bool,
    has_app_service: bool,
) -> Result<RuntimeMode> {
    if let Some(runtime) = config.runtime {
        return Ok(runtime);
    }

    match (has_package_json, has_app_service) {
        (true, false) => Ok(RuntimeMode::Native),
        (false, true) => Ok(RuntimeMode::Container),
        (true, true) => Err(DinopodError::RuntimeModeAmbiguous),
        (false, false) => Err(DinopodError::ProjectTypeUnknown),
    }
}

/// Resolves the npm/pnpm script to run for native dev.
///
/// # Errors
///
/// Returns [`DinopodError::DevScriptMissing`] when no suitable script exists.
pub fn resolve_dev_script(
    package_json: &Value,
    script_override: Option<&str>,
    configured_script: Option<&str>,
) -> Result<String> {
    if let Some(script) = script_override {
        return require_script(package_json, script);
    }
    if let Some(script) = configured_script {
        return require_script(package_json, script);
    }
    if script_exists(package_json, "dev:all") {
        return Ok("dev:all".to_owned());
    }
    if script_exists(package_json, "dev") {
        return Ok("dev".to_owned());
    }

    Err(DinopodError::DevScriptMissing {
        available: available_scripts(package_json),
    })
}

fn require_script(package_json: &Value, script: &str) -> Result<String> {
    if script_exists(package_json, script) {
        Ok(script.to_owned())
    } else {
        Err(DinopodError::DevScriptMissing {
            available: available_scripts(package_json),
        })
    }
}

fn script_exists(package_json: &Value, script: &str) -> bool {
    package_json
        .get("scripts")
        .and_then(Value::as_object)
        .is_some_and(|scripts| scripts.contains_key(script))
}

fn available_scripts(package_json: &Value) -> String {
    package_json
        .get("scripts")
        .and_then(Value::as_object)
        .map_or_else(String::new, |scripts| {
            let mut names = scripts.keys().cloned().collect::<Vec<_>>();
            names.sort_unstable();
            names.join(", ")
        })
}

fn default_app_port(config: &DinopodConfig, is_nextjs: bool) -> u16 {
    if let Some(port) = config.native.app_port {
        return port;
    }
    if is_nextjs {
        3000
    } else {
        config.app.internal_port
    }
}

/// Builds a project profile from repository signals.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when detection or script resolution fails.
pub fn build_project_profile(
    config: &DinopodConfig,
    repo_root: &Path,
    compose_service_names: &[String],
    script_override: Option<&str>,
    fs: &impl DetectFs,
) -> Result<ProjectProfile> {
    let package_json_path = package_json_path(repo_root);
    let has_package_json = fs.file_exists(&package_json_path);
    let has_app_service =
        compose_has_app_service(compose_service_names, config.app.service.as_str());
    let runtime = resolve_runtime_mode(config, has_package_json, has_app_service)?;

    match runtime {
        RuntimeMode::Container => Ok(ProjectProfile {
            runtime,
            package_manager: None,
            dev_script: None,
            is_nextjs: false,
            default_app_port: config.app.internal_port,
        }),
        RuntimeMode::Native => {
            let contents = fs.read_to_string(&package_json_path).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    DinopodError::PackageJsonMissing
                } else {
                    DinopodError::Io(error)
                }
            })?;
            let package_json = parse_package_json(&contents)?;
            let is_nextjs = is_nextjs_project(&package_json);
            let dev_script = resolve_dev_script(
                &package_json,
                script_override,
                config.native.dev_script.as_deref(),
            )?;
            Ok(ProjectProfile {
                runtime,
                package_manager: detect_package_manager(repo_root, fs),
                dev_script: Some(dev_script),
                is_nextjs,
                default_app_port: default_app_port(config, is_nextjs),
            })
        }
    }
}
