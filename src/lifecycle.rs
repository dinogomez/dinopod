//! High-level environment lifecycle orchestration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::compose::render_override;
use crate::compose::ComposeInspection;
use crate::config::DinopodConfig;
use crate::errors::{DinopodError, Result};
use crate::names::{derive_names, EnvironmentNames};
use crate::routes::render_route;
use crate::state::{EnvironmentRecord, EnvironmentStatus, StateStore};

/// Side-effect boundary used by lifecycle orchestration.
pub trait LifecyclePorts {
    /// Ensures a Git worktree exists for the environment.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when Git worktree setup fails.
    fn ensure_worktree(
        &self,
        repo_root: &Path,
        worktree_path: &Path,
        branch: &str,
        default_branch: &str,
    ) -> Result<()>;

    /// Writes the generated Compose override.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the file cannot be written.
    fn write_compose_override(&self, path: &Path, contents: &str) -> Result<()>;

    /// Writes the generated proxy route.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the route cannot be written.
    fn write_route(&self, path: &Path, contents: &str) -> Result<()>;

    /// Removes the generated proxy route.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the route cannot be removed.
    fn remove_route(&self, path: &Path) -> Result<()>;

    /// Ensures the shared proxy is available.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the proxy cannot be started or repaired.
    fn ensure_proxy(&self) -> Result<()>;

    /// Starts the app Compose project.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when Compose startup fails.
    fn compose_up(&self, project: &str, compose_files: &[PathBuf]) -> Result<ComposeInspection>;

    /// Stops the app Compose project.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when Compose stop fails.
    fn compose_stop(&self, project: &str, compose_files: &[PathBuf]) -> Result<()>;

    /// Removes the app Compose project.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when Compose down fails.
    fn compose_down(&self, project: &str, compose_files: &[PathBuf], volumes: bool) -> Result<()>;

    /// Removes a Git worktree.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when worktree removal fails.
    fn remove_worktree(&self, repo_root: &Path, path: &Path) -> Result<()>;

    /// Returns whether the Compose project is currently running.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when Docker state cannot be inspected.
    fn project_is_running(&self, project: &str) -> Result<bool>;
}

/// User-facing summary printed after `dinopod dev`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevSummary {
    /// Resolved worktree path.
    pub worktree_path: PathBuf,
    /// Docker Compose project name.
    pub project: String,
    /// Local URL routed through the proxy.
    pub url: String,
    /// Non-fatal Compose warnings discovered during startup.
    pub warnings: Vec<crate::compose::ComposeWarning>,
}

/// Coordinates Dinopod lifecycle commands.
#[derive(Debug)]
pub struct LifecycleManager<'a, P, S> {
    config: DinopodConfig,
    repo_name: String,
    repo_root: PathBuf,
    config_root: PathBuf,
    ports: &'a P,
    state: &'a S,
}

impl<'a, P, S> LifecycleManager<'a, P, S>
where
    P: LifecyclePorts,
    S: StateStore,
{
    /// Creates a lifecycle manager.
    #[must_use]
    pub fn new(
        config: DinopodConfig,
        repo_name: impl Into<String>,
        repo_root: impl Into<PathBuf>,
        config_root: impl Into<PathBuf>,
        ports: &'a P,
        state: &'a S,
    ) -> Self {
        Self {
            config,
            repo_name: repo_name.into(),
            repo_root: repo_root.into(),
            config_root: config_root.into(),
            ports,
            state,
        }
    }

    /// Creates or refreshes an environment for `ticket`.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when any lifecycle step fails.
    pub fn dev(&self, ticket: &str) -> Result<DevSummary> {
        let spec = self.environment_spec(ticket)?;
        self.ports.ensure_worktree(
            &self.repo_root,
            &spec.record.worktree_path,
            spec.names.ticket_slug.as_str(),
            &self.config.app.default_branch,
        )?;
        self.ports.write_compose_override(
            &spec.compose_override_path,
            &render_override(&self.config, &spec.names),
        )?;
        self.ports.ensure_proxy()?;
        self.ports.write_route(
            &spec.record.route_path,
            &render_route(&self.config, &spec.names),
        )?;

        let compose_files = spec.record.compose_files();
        let inspection = match self.ports.compose_up(&spec.record.project, &compose_files) {
            Ok(inspection) => inspection,
            Err(error) => {
                let _ = self.ports.remove_route(&spec.record.route_path);
                return Err(error);
            }
        };

        if let Err(error) = self.upsert_record(spec.record.clone()) {
            return Err(DinopodError::StatePersistFailed {
                project: spec.record.project.clone(),
                source: Box::new(error),
            });
        }

        Ok(DevSummary {
            worktree_path: spec.record.worktree_path,
            project: spec.record.project,
            url: spec.record.url,
            warnings: inspection.warnings().to_vec(),
        })
    }

    /// Lists tracked environments without mutating state.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when state cannot be read.
    pub fn list(&self) -> Result<Vec<EnvironmentRecord>> {
        Ok(self.state.load()?.into_values().collect())
    }

    /// Reconciles tracked environments with Docker and persists updated status.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when state or Docker inspection fails.
    pub fn list_reconciled(&self) -> Result<Vec<EnvironmentRecord>> {
        let mut records = self.state.load()?;
        for record in records.values_mut() {
            if record.status == EnvironmentStatus::Running
                && !self.ports.project_is_running(&record.project)?
            {
                record.status = EnvironmentStatus::Stale;
            }
        }
        let values = records.into_values().collect::<Vec<_>>();
        self.state.save(values.clone())?;
        Ok(values)
    }

    /// Stops an environment while retaining containers and volumes.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the environment is unknown or Compose fails.
    pub fn stop(&self, ticket: &str) -> Result<()> {
        self.update_record(ticket, |record| {
            self.ports
                .compose_stop(&record.project, &record.compose_files())?;
            record.status = EnvironmentStatus::Stopped;
            Ok(())
        })
    }

    /// Downs an environment and removes its route.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the environment is unknown, Compose fails,
    /// or the route cannot be removed.
    pub fn down(&self, ticket: &str, volumes: bool) -> Result<()> {
        self.update_record(ticket, |record| {
            self.ports
                .compose_down(&record.project, &record.compose_files(), volumes)?;
            self.ports.remove_route(&record.route_path)?;
            record.status = EnvironmentStatus::Down;
            Ok(())
        })
    }

    /// Removes an environment and its worktree after confirmation.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::ConfirmationRequired`] when `confirmed` is false.
    pub fn rm(&self, ticket: &str, confirmed: bool) -> Result<()> {
        if !confirmed {
            return Err(DinopodError::ConfirmationRequired {
                ticket: ticket.to_owned(),
            });
        }

        let project = self.project_for_ticket(ticket)?;
        let mut records = self.state.load()?;
        let Some(record) = records.remove(&project) else {
            return Err(DinopodError::EnvironmentNotFound {
                ticket: ticket.to_owned(),
            });
        };

        self.ports
            .compose_down(&record.project, &record.compose_files(), false)?;
        self.ports.remove_route(&record.route_path)?;
        self.ports
            .remove_worktree(&self.repo_root, &record.worktree_path)?;
        self.save_records(records)
    }

    fn environment_spec(&self, ticket: &str) -> Result<EnvironmentSpec> {
        let names = derive_names(&self.repo_name, ticket, &self.repo_root, &self.config)?;
        let project = names.project.as_str().to_owned();
        let host = names.host.as_str().to_owned();
        let url = environment_url(&host, self.config.proxy.http_port);
        let worktree_path = names.worktree_path.as_path().to_path_buf();
        let route_path = self
            .config_root
            .join("proxy")
            .join("dynamic")
            .join(format!("{project}.toml"));
        let compose_override_path = worktree_path.join(".dinopod").join("compose.override.yml");
        let user_compose_path = worktree_path.join(&self.config.app.compose_file);

        Ok(EnvironmentSpec {
            record: EnvironmentRecord {
                project,
                ticket: ticket.to_owned(),
                host,
                url,
                worktree_path,
                route_path,
                user_compose_path: Some(user_compose_path),
                compose_override_path: Some(compose_override_path.clone()),
                status: EnvironmentStatus::Running,
            },
            compose_override_path,
            names,
        })
    }

    fn upsert_record(&self, record: EnvironmentRecord) -> Result<()> {
        let mut records = self.state.load()?;
        records.insert(record.project.clone(), record);
        self.save_records(records)
    }

    fn update_record<F>(&self, ticket: &str, update: F) -> Result<()>
    where
        F: FnOnce(&mut EnvironmentRecord) -> Result<()>,
    {
        let project = self.project_for_ticket(ticket)?;
        let mut records = self.state.load()?;
        let Some(record) = records.get_mut(&project) else {
            return Err(DinopodError::EnvironmentNotFound {
                ticket: ticket.to_owned(),
            });
        };
        update(record)?;
        self.save_records(records)
    }

    fn project_for_ticket(&self, ticket: &str) -> Result<String> {
        Ok(
            derive_names(&self.repo_name, ticket, &self.repo_root, &self.config)?
                .project
                .as_str()
                .to_owned(),
        )
    }

    fn save_records(&self, records: BTreeMap<String, EnvironmentRecord>) -> Result<()> {
        self.state.save(records.into_values().collect())
    }
}

fn environment_url(host: &str, http_port: u16) -> String {
    if http_port == 80 {
        format!("http://{host}")
    } else {
        format!("http://{host}:{http_port}")
    }
}

#[derive(Debug)]
struct EnvironmentSpec {
    record: EnvironmentRecord,
    compose_override_path: PathBuf,
    names: EnvironmentNames,
}
