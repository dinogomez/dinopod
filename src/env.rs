//! Environment file copy, port allocation, and overlay generation for native dev.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{should_copy_env_file, SettingsConfig};
use crate::detect::PackageManager;
use crate::errors::{DinopodError, Result};

const APP_PORT_RANGE: (u16, u16) = (31_000, 31_999);
const POSTGRES_PORT_RANGE: (u16, u16) = (54_000, 54_999);
const REDIS_PORT_RANGE: (u16, u16) = (63_000, 63_999);

/// Env files copied from the primary repo into a new worktree.
pub const ENV_FILE_NAMES: [&str; 4] = [
    ".env",
    ".env.local",
    ".env.development",
    ".env.development.local",
];

/// Returns true when `name` is an env file copied into worktrees.
#[must_use]
pub fn is_dotenv_file_name(name: &str) -> bool {
    name.contains(".env")
}

/// Per-ticket host port assignments for native dev.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortPlan {
    /// Host port for the app process.
    pub app_host_port: u16,
    /// Host port published for Postgres when present in Compose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres_host_port: Option<u16>,
    /// Host port published for Redis when present in Compose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_host_port: Option<u16>,
}

/// Inputs used to render a ticket-specific env overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayContext {
    /// Public app URL for the ticket hostname.
    pub ticket_url: String,
    /// Port assignments for the ticket.
    pub port_plan: PortPlan,
}

/// Filesystem boundary for env file operations.
pub trait EnvFs {
    /// Returns true when `path` exists.
    fn path_exists(&self, path: &Path) -> bool;

    /// Returns true when `path` is a directory.
    fn dir_exists(&self, path: &Path) -> bool;

    /// Returns true when `path` is a symlink.
    fn is_symlink(&self, path: &Path) -> bool;

    /// Reads a UTF-8 file.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;

    /// Creates a directory and any parents.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Writes a file with the requested Unix mode when supported.
    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> io::Result<()>;

    /// Copies a regular file without following symlinks.
    fn copy_regular_file(&self, from: &Path, to: &Path, mode: u32) -> io::Result<()>;

    /// Lists regular env files in `root` whose names contain `.env`.
    fn list_dotenv_files(&self, root: &Path) -> io::Result<Vec<PathBuf>>;
}

/// Production filesystem adapter for env operations.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdEnvFs;

impl EnvFs for StdEnvFs {
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn dir_exists(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_symlink(&self, path: &Path) -> bool {
        path.symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)?;
        set_mode(path, 0o700)
    }

    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            self.create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        set_mode(path, mode)
    }

    fn copy_regular_file(&self, from: &Path, to: &Path, mode: u32) -> io::Result<()> {
        if self.is_symlink(from) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to copy symlink env file",
            ));
        }
        let contents = self.read_to_string(from)?;
        self.write_file(to, &contents, mode)
    }

    fn list_dotenv_files(&self, root: &Path) -> io::Result<Vec<PathBuf>> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if !is_dotenv_file_name(name) {
                continue;
            }
            if file_type.is_symlink() || file_type.is_file() {
                files.push(entry.path());
            }
        }
        files.sort();
        Ok(files)
    }
}

/// Port bind probe used during deterministic port allocation.
pub trait PortBinder {
    /// Returns true when the host port can be bound locally.
    fn can_bind(&self, port: u16) -> bool;
}

/// Production TCP bind probe for port allocation.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdPortBinder;

impl PortBinder for StdPortBinder {
    fn can_bind(&self, port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }
}

/// Returns the Dinopod artifact directory inside a worktree.
#[must_use]
pub fn dinopod_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(".dinopod")
}

/// Returns the generated env overlay path for a worktree.
#[must_use]
pub fn env_overlay_path(worktree_root: &Path) -> PathBuf {
    dinopod_dir(worktree_root).join("env.overlay")
}

/// Copies all dotenv files from `source_root` into a worktree.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when symlink env files are encountered or I/O fails.
pub fn copy_env_files_on_create(
    source_root: &Path,
    worktree_root: &Path,
    settings: &SettingsConfig,
    fs: &impl EnvFs,
) -> Result<()> {
    sync_env_files(source_root, worktree_root, settings, true, fs)
}

/// Copies dotenv files from `source_root` that are missing in the worktree.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when symlink env files are encountered or I/O fails.
pub fn sync_missing_env_files(
    source_root: &Path,
    worktree_root: &Path,
    settings: &SettingsConfig,
    fs: &impl EnvFs,
) -> Result<()> {
    sync_env_files(source_root, worktree_root, settings, false, fs)
}

fn sync_env_files(
    source_root: &Path,
    worktree_root: &Path,
    settings: &SettingsConfig,
    overwrite_existing: bool,
    fs: &impl EnvFs,
) -> Result<()> {
    let dinopod = dinopod_dir(worktree_root);
    fs.create_dir_all(&dinopod).map_err(DinopodError::Io)?;

    for source in fs.list_dotenv_files(source_root)? {
        if fs.is_symlink(&source) {
            return Err(DinopodError::EnvSymlinkRejected { path: source });
        }
        let Some(file_name) = source.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !should_copy_env_file(file_name, &settings.env_skip_patterns) {
            continue;
        }
        let destination = worktree_root.join(file_name);
        if !overwrite_existing && fs.path_exists(&destination) {
            continue;
        }
        fs.copy_regular_file(&source, &destination, 0o600)
            .map_err(DinopodError::Io)?;
    }

    Ok(())
}

/// Merges missing env keys from the primary repo into worktree copies.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when env files cannot be read or written.
pub fn refresh_env_files(source_root: &Path, worktree_root: &Path, fs: &impl EnvFs) -> Result<()> {
    for source in fs.list_dotenv_files(source_root)? {
        if fs.is_symlink(&source) {
            return Err(DinopodError::EnvSymlinkRejected { path: source });
        }
        let Some(file_name) = source.file_name() else {
            continue;
        };
        let worktree_path = worktree_root.join(file_name);

        let primary_vars = parse_env_file(&fs.read_to_string(&source).map_err(DinopodError::Io)?);
        let mut worktree_vars = if fs.path_exists(&worktree_path) {
            parse_env_file(
                &fs.read_to_string(&worktree_path)
                    .map_err(DinopodError::Io)?,
            )
        } else {
            BTreeMap::new()
        };

        let mut changed = false;
        for (key, value) in primary_vars {
            if let std::collections::btree_map::Entry::Vacant(e) = worktree_vars.entry(key) {
                e.insert(value);
                changed = true;
            }
        }

        if changed {
            fs.write_file(&worktree_path, &render_env_file(&worktree_vars), 0o600)
                .map_err(DinopodError::Io)?;
        }
    }

    Ok(())
}

/// Returns true when dependency installation should run for a worktree.
#[must_use]
pub fn should_install_dependencies(
    worktree_root: &Path,
    no_install: bool,
    fs: &impl EnvFs,
) -> bool {
    !no_install && !fs.dir_exists(&worktree_root.join("node_modules"))
}

/// Returns the install command for the detected package manager.
#[must_use]
pub fn install_program(package_manager: PackageManager) -> &'static str {
    match package_manager {
        PackageManager::Pnpm => "pnpm",
        PackageManager::Npm => "npm",
    }
}

/// Returns install arguments for the detected package manager.
#[must_use]
pub fn install_arguments(package_manager: PackageManager) -> &'static [&'static str] {
    match package_manager {
        PackageManager::Pnpm => &["install"],
        PackageManager::Npm => &["ci"],
    }
}

/// Allocates deterministic per-ticket ports with collision probing.
///
/// # Errors
///
/// Returns [`DinopodError::PortRangeExhausted`] when no free port remains in a required range.
pub fn allocate_port_plan(
    repo_slug: &str,
    ticket_slug: &str,
    infra_services: &[String],
    binder: &impl PortBinder,
) -> Result<PortPlan> {
    let seed = port_seed(repo_slug, ticket_slug);
    let needs_postgres = infra_services
        .iter()
        .any(|service| is_postgres_service(service));
    let needs_redis = infra_services.iter().any(|service| service == "redis");

    Ok(PortPlan {
        app_host_port: allocate_in_range(seed, 0, APP_PORT_RANGE, "app", binder)?,
        postgres_host_port: if needs_postgres {
            Some(allocate_in_range(
                seed,
                1,
                POSTGRES_PORT_RANGE,
                "postgres",
                binder,
            )?)
        } else {
            None
        },
        redis_host_port: if needs_redis {
            Some(allocate_in_range(
                seed,
                2,
                REDIS_PORT_RANGE,
                "redis",
                binder,
            )?)
        } else {
            None
        },
    })
}

/// Renders the ticket env overlay file contents.
#[must_use]
pub fn render_env_overlay(
    context: &OverlayContext,
    existing_env: &BTreeMap<String, String>,
) -> String {
    let mut lines = Vec::new();
    let postgres_port = context.port_plan.postgres_host_port;
    let redis_port = context.port_plan.redis_host_port;

    if let Some(port) = postgres_port {
        for key in ["DATABASE_URL", "DIRECT_URL"] {
            if let Some(value) = existing_env.get(key) {
                lines.push(format!("{key}={}", replace_url_port(value, port)));
            } else {
                lines.push(format!(
                    "{key}=postgresql://postgres:postgres@localhost:{port}/postgres"
                ));
            }
        }
    }

    if let Some(port) = redis_port {
        if let Some(value) = existing_env.get("REDIS_URL") {
            lines.push(format!("REDIS_URL={}", replace_url_port(value, port)));
        } else {
            lines.push(format!("REDIS_URL=redis://localhost:{port}"));
        }
    }

    lines.push(format!("NEXT_PUBLIC_APP_URL={}", context.ticket_url));
    lines.push(format!("BETTER_AUTH_URL={}", context.ticket_url));
    lines.push(format!("PORT={}", context.port_plan.app_host_port));
    format!("{}\n", lines.join("\n"))
}

/// Writes the env overlay file for a worktree.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when the overlay cannot be written.
pub fn write_env_overlay(
    worktree_root: &Path,
    context: &OverlayContext,
    existing_env: &BTreeMap<String, String>,
    fs: &impl EnvFs,
) -> Result<PathBuf> {
    let path = env_overlay_path(worktree_root);
    let contents = render_env_overlay(context, existing_env);
    fs.write_file(&path, &contents, 0o600)
        .map_err(DinopodError::Io)?;
    Ok(path)
}

/// Parses dotenv-style files into a deterministic map.
#[must_use]
pub fn parse_env_file(contents: &str) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        vars.insert(key.trim().to_owned(), trim_env_value(value));
    }
    vars
}

/// Loads env vars from copied worktree files plus overlay; overlay wins on conflicts.
///
/// # Errors
///
/// Returns a recoverable Dinopod error when env files cannot be read.
pub fn load_merged_env(
    worktree_root: &Path,
    overlay_path: &Path,
    fs: &impl EnvFs,
) -> Result<BTreeMap<String, String>> {
    let mut merged = BTreeMap::new();
    for path in fs.list_dotenv_files(worktree_root)? {
        merged.extend(parse_env_file(
            &fs.read_to_string(&path).map_err(DinopodError::Io)?,
        ));
    }
    if fs.path_exists(overlay_path) {
        merged.extend(parse_env_file(
            &fs.read_to_string(overlay_path).map_err(DinopodError::Io)?,
        ));
    }
    Ok(merged)
}

fn render_env_file(vars: &BTreeMap<String, String>) -> String {
    vars.iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn trim_env_value(value: &str) -> String {
    let trimmed = value.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed[1..trimmed.len() - 1].to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn is_postgres_service(service: &str) -> bool {
    service == "db" || service == "postgres" || service.starts_with("postgres")
}

fn port_seed(repo_slug: &str, ticket_slug: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    repo_slug.hash(&mut hasher);
    ticket_slug.hash(&mut hasher);
    hasher.finish()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "port ranges are bounded to u16 and span fits in u32"
)]
fn allocate_in_range(
    seed: u64,
    salt: u64,
    range: (u16, u16),
    service: &str,
    binder: &impl PortBinder,
) -> Result<u16> {
    let (start, end) = range;
    let span = u32::from(end - start + 1);
    let base = start + ((seed.wrapping_add(salt)) as u32 % span) as u16;

    for offset in 0..span {
        let port = start + ((u32::from(base - start) + offset) % span) as u16;
        if binder.can_bind(port) {
            return Ok(port);
        }
    }

    Err(DinopodError::PortRangeExhausted {
        service: service.to_owned(),
        range: format!("{start}-{end}"),
    })
}

fn replace_url_port(url: &str, new_port: u16) -> String {
    if let Some(at_idx) = url.rfind('@') {
        let authority = &url[at_idx + 1..];
        if let Some(colon) = authority.find(':') {
            let host = &authority[..colon];
            let suffix = authority[colon + 1..]
                .find('/')
                .map_or("", |slash| &authority[colon + 1 + slash..]);
            return format!("{}@{host}:{new_port}{suffix}", &url[..at_idx]);
        }
    }

    if let Some(scheme_end) = url.find("://") {
        let scheme = &url[..scheme_end];
        let rest = &url[scheme_end + 3..];
        if let Some(colon) = rest.find(':') {
            let host = &rest[..colon];
            let suffix = rest[colon + 1..]
                .find('/')
                .map_or("", |slash| &rest[colon + 1 + slash..]);
            return format!("{scheme}://{host}:{new_port}{suffix}");
        }
    }

    url.to_owned()
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_url_port_should_rewrite_postgres_and_redis_urls() {
        let postgres = "postgresql://user:pass@localhost:5432/app?schema=public";
        assert_eq!(
            replace_url_port(postgres, 54_321),
            "postgresql://user:pass@localhost:54321/app?schema=public"
        );

        let redis = "redis://localhost:6379";
        assert_eq!(replace_url_port(redis, 63_210), "redis://localhost:63210");
    }
}
