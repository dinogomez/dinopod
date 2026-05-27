//! Local Dinopod environment state.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::Result;
use crate::fs::{AtomicWriter, StdAtomicFileSystem};

/// Lifecycle status cached for an environment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentStatus {
    /// Docker project is expected to be running.
    Running,
    /// Containers were stopped but retained.
    Stopped,
    /// Containers/networks were removed while state was retained.
    Down,
    /// State no longer matches Docker/Git reality.
    Stale,
}

/// Cached local state for one Dinopod environment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentRecord {
    /// Compose project key.
    pub project: String,
    /// User-provided ticket identifier.
    pub ticket: String,
    /// Local hostname.
    pub host: String,
    /// User-facing URL.
    pub url: String,
    /// Worktree path.
    pub worktree_path: PathBuf,
    /// Generated route file path.
    pub route_path: PathBuf,
    /// User Compose file path inside the worktree.
    #[serde(default)]
    pub user_compose_path: Option<PathBuf>,
    /// Dinopod Compose override path inside the worktree.
    #[serde(default)]
    pub compose_override_path: Option<PathBuf>,
    /// Cached lifecycle status.
    pub status: EnvironmentStatus,
}

impl EnvironmentRecord {
    /// Returns the Compose file pair used for Docker commands.
    #[must_use]
    pub fn compose_files(&self) -> Vec<PathBuf> {
        let user = self
            .user_compose_path
            .clone()
            .unwrap_or_else(|| self.worktree_path.join("docker-compose.yml"));
        let override_file = self
            .compose_override_path
            .clone()
            .unwrap_or_else(|| self.worktree_path.join(".dinopod/compose.override.yml"));
        vec![user, override_file]
    }
}

/// Local state persistence boundary.
pub trait StateStore {
    /// Loads known environment records keyed by Compose project.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot be read or decoded.
    fn load(&self) -> Result<BTreeMap<String, EnvironmentRecord>>;

    /// Replaces the state with `records`.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot be encoded or written.
    fn save(&self, records: Vec<EnvironmentRecord>) -> Result<()>;
}

/// In-memory state store for tests.
#[derive(Debug, Default)]
pub struct InMemoryStateStore {
    records: RefCell<BTreeMap<String, EnvironmentRecord>>,
}

impl StateStore for InMemoryStateStore {
    fn load(&self) -> Result<BTreeMap<String, EnvironmentRecord>> {
        Ok(self.records.borrow().clone())
    }

    fn save(&self, records: Vec<EnvironmentRecord>) -> Result<()> {
        self.records.borrow_mut().clear();
        self.records.borrow_mut().extend(
            records
                .into_iter()
                .map(|record| (record.project.clone(), record)),
        );
        Ok(())
    }
}

/// TOML state store backed by an atomic write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStateStore {
    path: PathBuf,
}

impl FileStateStore {
    /// Creates a file state store at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the state file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl StateStore for FileStateStore {
    fn load(&self) -> Result<BTreeMap<String, EnvironmentRecord>> {
        match fs::read_to_string(&self.path) {
            Ok(contents) => {
                let state = toml::from_str::<StateFile>(&contents)?;
                Ok(state
                    .environments
                    .into_iter()
                    .map(|record| (record.project.clone(), record))
                    .collect())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error.into()),
        }
    }

    fn save(&self, records: Vec<EnvironmentRecord>) -> Result<()> {
        let state = StateFile {
            environments: records,
        };
        let contents = toml::to_string_pretty(&state)?;
        let mut writer = AtomicWriter::new(StdAtomicFileSystem);
        writer.write_atomic(&self.path, &contents)?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StateFile {
    environments: Vec<EnvironmentRecord>,
}
