//! High-level environment lifecycle orchestration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::compose::{
    filter_managed_port_warnings, infra_service_names, render_infra_override, render_override,
    render_pod_override, validate_infra_host_ports, ComposeInspection,
};
use crate::config::{DinopodConfig, RuntimeMode};
use crate::detect::{build_project_profile, PackageManager, ProjectProfile, StdDetectFs};
use crate::env::{
    allocate_port_plan, install_program, load_merged_env, write_env_overlay, OverlayContext,
    StdEnvFs, StdPortBinder,
};
use crate::errors::{DinopodError, Result};
use crate::git::WorktreeAction;
use crate::names::{derive_names, repo_slug, EnvironmentNames};
use crate::process::{
    ensure_dev_process_running, terminate_listeners_on_port, NativeDevLaunch, StdProcessFs,
};
use crate::routes::{render_route, render_route_with_upstream, RouteUpstream};
use crate::state::{EnvironmentRecord, EnvironmentStatus, StateStore};
use crate::ui::{lifecycle_fail, lifecycle_finalize, lifecycle_progress, Ui};

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
    ) -> Result<WorktreeAction>;

    /// Inspects the user-owned Compose file before Dinopod overrides are applied.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the Compose file is missing or invalid.
    fn inspect_user_compose(&self, user_file: &Path) -> Result<ComposeInspection>;

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

    /// Copies env files into a newly created worktree.
    fn copy_env_on_create(&self, source_root: &Path, worktree_root: &Path) -> Result<()>;

    /// Copies dotenv files from the source directory when absent in the worktree.
    fn sync_missing_env(&self, source_root: &Path, worktree_root: &Path) -> Result<()>;

    /// Merges missing env keys from the source directory into the worktree.
    fn refresh_env(&self, source_root: &Path, worktree_root: &Path) -> Result<()>;

    /// Installs dependencies in a worktree when native mode requires it.
    fn install_dependencies(
        &self,
        worktree_root: &Path,
        package_manager: PackageManager,
    ) -> Result<()>;

    /// Inspects the user-owned Compose model including optional user override files.
    fn inspect_user_compose_merged(&self, user_file: &Path) -> Result<(ComposeInspection, String)>;

    /// Inspects the effective Compose stack including the Dinopod override file.
    fn inspect_compose_stack(
        &self,
        user_file: &Path,
        dinopod_override: &Path,
    ) -> Result<(ComposeInspection, String)>;

    /// Starts only the requested infra Compose services.
    fn compose_up_infra(
        &self,
        project: &str,
        compose_files: &[PathBuf],
        services: &[String],
    ) -> Result<ComposeInspection>;

    /// Starts the full Compose project for a pod.
    fn compose_up_all(&self, project: &str, compose_files: &[PathBuf])
        -> Result<ComposeInspection>;

    /// Runs a setup command in the worktree with the provided environment.
    fn run_setup_command(
        &self,
        worktree_root: &Path,
        command: &str,
        env: &[(String, String)],
    ) -> Result<()>;

    /// Spawns the native dev process in a worktree.
    fn spawn_dev_process(
        &self,
        worktree_root: &Path,
        package_manager: PackageManager,
        script: &str,
        env: &[(String, String)],
    ) -> Result<u32>;

    /// Stops the native dev process in a worktree.
    fn stop_dev_process(&self, worktree_root: &Path) -> Result<()>;
}

/// Options for `dinopod dev`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DevOptions {
    /// npm/pnpm script override.
    pub script: Option<String>,
    /// Merge missing env keys from the primary repo.
    pub refresh_env: bool,
    /// Skip dependency installation on new worktrees.
    pub no_install: bool,
    /// Run the native dev script in the background instead of the foreground terminal.
    pub detach: bool,
}

/// User-facing summary printed after `dinopod new`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PodSummary {
    /// Resolved worktree path.
    pub worktree_path: PathBuf,
    /// Docker Compose project name.
    pub project: String,
    /// Local URL routed through the proxy when configured.
    pub url: String,
    /// Non-fatal Compose warnings discovered during startup.
    pub warnings: Vec<crate::compose::ComposeWarning>,
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
    /// Foreground native dev launch parameters.
    pub native_dev: Option<NativeDevLaunch>,
    /// Background native dev PID when `--detach` is used.
    pub background_pid: Option<u32>,
}

/// Coordinates Dinopod lifecycle commands.
#[derive(Debug)]
pub struct LifecycleManager<'a, P, S> {
    config: DinopodConfig,
    repo_name: String,
    repo_root: PathBuf,
    env_source_root: PathBuf,
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
        env_source_root: impl Into<PathBuf>,
        config_root: impl Into<PathBuf>,
        ports: &'a P,
        state: &'a S,
    ) -> Self {
        Self {
            config,
            repo_name: repo_name.into(),
            repo_root: repo_root.into(),
            env_source_root: env_source_root.into(),
            config_root: config_root.into(),
            ports,
            state,
        }
    }

    /// Provisions a pod: worktree, isolated compose, and configured setup commands.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when any lifecycle step fails.
    pub fn new_pod(&self, ticket: &str, mut ui: Option<&mut dyn Ui>) -> Result<PodSummary> {
        lifecycle_progress(&mut ui, "provisioning pod");
        let result = self.new_pod_steps(ticket, &mut ui);
        match &result {
            Ok(_) => lifecycle_finalize(&mut ui),
            Err(_) => lifecycle_fail(&mut ui),
        }
        result
    }

    #[expect(
        clippy::too_many_lines,
        reason = "pod provisioning is an explicit ordered sequence"
    )]
    fn new_pod_steps(&self, ticket: &str, ui: &mut Option<&mut dyn Ui>) -> Result<PodSummary> {
        lifecycle_progress(ui, "planning pod");
        let spec = self.environment_spec(ticket)?;
        lifecycle_progress(ui, "ensuring git worktree");
        let worktree_action = self.ports.ensure_worktree(
            &self.repo_root,
            &spec.record.worktree_path,
            spec.names.ticket_slug.as_str(),
            self.config.default_branch(),
        )?;
        let compose_inspection = self.ports.inspect_user_compose(&spec.user_compose_path)?;

        if worktree_action == WorktreeAction::Created && self.config.settings.copy_env {
            lifecycle_progress(ui, "copying env files into new worktree");
            self.ports
                .copy_env_on_create(&self.repo_root, &spec.record.worktree_path)?;
        } else if worktree_action != WorktreeAction::Created {
            lifecycle_progress(ui, "syncing env files");
            self.ports
                .sync_missing_env(&self.repo_root, &spec.record.worktree_path)?;
        }

        lifecycle_progress(ui, "allocating ports");
        let repo_slug = repo_slug(&self.repo_name)?;
        let port_plan = allocate_port_plan(
            &repo_slug,
            spec.names.ticket_slug.as_str(),
            compose_inspection.service_names(),
            &StdPortBinder,
        )?;

        let (user_merged_inspection, user_compose_json) = self
            .ports
            .inspect_user_compose_merged(&spec.user_compose_path)?;
        let user_compose_value: serde_json::Value = serde_json::from_str(&user_compose_json)?;
        lifecycle_progress(ui, "preparing compose override");
        self.ports.write_compose_override(
            &spec.compose_override_path,
            &render_pod_override(
                &user_compose_value,
                &self.config,
                &spec.names,
                &port_plan,
                user_merged_inspection.service_names(),
                user_merged_inspection.attach_implicit_default_network(),
            ),
        )?;

        let (merged_inspection, compose_json) = self
            .ports
            .inspect_compose_stack(&spec.user_compose_path, &spec.compose_override_path)?;
        let compose_value: serde_json::Value = serde_json::from_str(&compose_json)?;
        validate_infra_host_ports(&compose_value, &self.config.app.service, &port_plan)?;

        lifecycle_progress(ui, "generating env overlay");
        let merged_env = load_merged_env(
            &spec.record.worktree_path,
            &spec.record.worktree_path.join(".dinopod/env.overlay"),
            &StdEnvFs,
        )?;
        let overlay_path = write_env_overlay(
            &spec.record.worktree_path,
            &OverlayContext {
                ticket_url: spec.record.url.clone(),
                port_plan: port_plan.clone(),
            },
            &merged_env,
            &StdEnvFs,
        )?;
        let spawn_env = load_merged_env(&spec.record.worktree_path, &overlay_path, &StdEnvFs)?
            .into_iter()
            .collect::<Vec<_>>();

        lifecycle_progress(ui, "starting shared proxy");
        self.ports.ensure_proxy()?;

        let app_in_compose = merged_inspection
            .service_names()
            .iter()
            .any(|service| service == &self.config.app.service);
        if app_in_compose {
            lifecycle_progress(ui, "registering proxy route");
            let route = render_route(&self.config, &spec.names);
            self.ports.write_route(&spec.record.route_path, &route)?;
        } else {
            lifecycle_progress(ui, "registering proxy route to host");
            let route = render_route_with_upstream(
                &spec.names,
                &RouteUpstream::HostGateway {
                    port: port_plan.app_host_port,
                },
            );
            self.ports.write_route(&spec.record.route_path, &route)?;
        }

        let compose_files = spec.record.compose_files();
        lifecycle_progress(ui, "starting compose project");
        if let Err(error) = self
            .ports
            .compose_up_all(&spec.record.project, &compose_files)
        {
            let _ = self.ports.remove_route(&spec.record.route_path);
            return Err(error);
        }

        let record_persisted = self
            .state
            .load()?
            .get(&spec.record.project)
            .is_some_and(|record| record.status == EnvironmentStatus::Running);

        if !record_persisted {
            for command in &self.config.setup.commands {
                lifecycle_progress(ui, &format!("running setup: {command}"));
                if let Err(error) =
                    self.ports
                        .run_setup_command(&spec.record.worktree_path, command, &spawn_env)
                {
                    let _ = self.ports.remove_route(&spec.record.route_path);
                    let _ = self
                        .ports
                        .compose_down(&spec.record.project, &compose_files, false);
                    return Err(error);
                }
            }
        }

        let mut record = spec.record;
        record.runtime_mode = None;
        record.app_host_port = Some(port_plan.app_host_port);
        record.env_overlay_path = Some(overlay_path);
        record.port_plan = Some(port_plan.clone());
        record.status = EnvironmentStatus::Running;

        if let Err(error) = self.upsert_record(record.clone()) {
            return Err(DinopodError::StatePersistFailed {
                project: record.project.clone(),
                source: Box::new(error),
            });
        }

        Ok(PodSummary {
            worktree_path: record.worktree_path,
            project: record.project,
            url: record.url,
            warnings: filter_managed_port_warnings(
                merged_inspection.warnings().to_vec(),
                &port_plan,
            ),
        })
    }

    /// Creates or refreshes an environment for `ticket`.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when any lifecycle step fails.
    pub fn dev(&self, ticket: &str) -> Result<DevSummary> {
        self.dev_with_options(ticket, &DevOptions::default(), None)
    }

    /// Creates or refreshes an environment for `ticket` with explicit dev options.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when any lifecycle step fails.
    pub fn dev_with_options(
        &self,
        ticket: &str,
        options: &DevOptions,
        mut ui: Option<&mut dyn Ui>,
    ) -> Result<DevSummary> {
        lifecycle_progress(&mut ui, "planning environment");
        let spec = self.environment_spec(ticket)?;
        lifecycle_progress(&mut ui, "ensuring git worktree");
        let worktree_action = self.ports.ensure_worktree(
            &self.repo_root,
            &spec.record.worktree_path,
            spec.names.ticket_slug.as_str(),
            self.config.default_branch(),
        )?;
        let compose_inspection = self.ports.inspect_user_compose(&spec.user_compose_path)?;
        let profile = self.resolve_project_profile(
            &spec,
            compose_inspection.service_names(),
            options.script.as_deref(),
        )?;

        match profile.runtime {
            RuntimeMode::Native => self.dev_native(
                spec,
                &profile,
                worktree_action,
                options,
                &compose_inspection,
                &mut ui,
            ),
            RuntimeMode::Container => self.dev_container(spec, &compose_inspection, &mut ui),
        }
    }

    fn dev_container(
        &self,
        spec: EnvironmentSpec,
        compose_inspection: &ComposeInspection,
        ui: &mut Option<&mut dyn Ui>,
    ) -> Result<DevSummary> {
        if !compose_inspection
            .service_names()
            .iter()
            .any(|service| service == &self.config.app.service)
        {
            return Err(DinopodError::ComposeServiceMissing {
                service: self.config.app.service.clone(),
            });
        }

        lifecycle_progress(ui, "writing compose override");
        self.ports.write_compose_override(
            &spec.compose_override_path,
            &render_override(
                &self.config,
                &spec.names,
                compose_inspection.attach_implicit_default_network(),
            ),
        )?;
        lifecycle_progress(ui, "starting shared proxy");
        self.ports.ensure_proxy()?;
        lifecycle_progress(ui, "registering proxy route");
        self.ports.write_route(
            &spec.record.route_path,
            &render_route(&self.config, &spec.names),
        )?;

        let compose_files = spec.record.compose_files();
        lifecycle_progress(ui, "starting compose project");
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
            native_dev: None,
            background_pid: None,
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "native dev orchestration is an explicit ordered sequence"
    )]
    fn dev_native(
        &self,
        mut spec: EnvironmentSpec,
        profile: &ProjectProfile,
        worktree_action: WorktreeAction,
        options: &DevOptions,
        inspection: &ComposeInspection,
        ui: &mut Option<&mut dyn Ui>,
    ) -> Result<DevSummary> {
        if worktree_action == WorktreeAction::Created && self.config.settings.copy_env {
            lifecycle_progress(ui, "copying env files into new worktree");
            self.ports
                .copy_env_on_create(&self.repo_root, &spec.record.worktree_path)?;
        } else if worktree_action != WorktreeAction::Created {
            lifecycle_progress(ui, "syncing env files");
            self.ports
                .sync_missing_env(&self.repo_root, &spec.record.worktree_path)?;
            if options.refresh_env {
                lifecycle_progress(ui, "refreshing env keys from source repo");
                self.ports
                    .refresh_env(&self.env_source_root, &spec.record.worktree_path)?;
            }
        }

        if crate::env::should_install_dependencies(
            &spec.record.worktree_path,
            options.no_install,
            &StdEnvFs,
        ) {
            if let Some(package_manager) = profile.package_manager {
                lifecycle_progress(ui, "installing dependencies");
                self.ports
                    .install_dependencies(&spec.record.worktree_path, package_manager)?;
            }
        }

        lifecycle_progress(ui, "allocating ports");
        let repo_slug = repo_slug(&self.repo_name)?;
        let port_plan = allocate_port_plan(
            &repo_slug,
            spec.names.ticket_slug.as_str(),
            inspection.service_names(),
            &StdPortBinder,
        )?;

        let (user_merged_inspection, user_compose_json) = self
            .ports
            .inspect_user_compose_merged(&spec.user_compose_path)?;
        let user_compose_value: serde_json::Value = serde_json::from_str(&user_compose_json)?;
        let infra_services = infra_service_names(
            user_merged_inspection.service_names(),
            &self.config.app.service,
        );
        lifecycle_progress(ui, "preparing compose override");
        self.ports.write_compose_override(
            &spec.compose_override_path,
            &render_infra_override(
                &user_compose_value,
                &self.config.app.service,
                &port_plan,
                &infra_services,
            ),
        )?;

        let (merged_inspection, compose_json) = self
            .ports
            .inspect_compose_stack(&spec.user_compose_path, &spec.compose_override_path)?;
        let compose_value: serde_json::Value = serde_json::from_str(&compose_json)?;
        validate_infra_host_ports(&compose_value, &self.config.app.service, &port_plan)?;

        lifecycle_progress(ui, "generating env overlay");
        let merged_env = load_merged_env(
            &spec.record.worktree_path,
            &spec.record.worktree_path.join(".dinopod/env.overlay"),
            &StdEnvFs,
        )?;
        let overlay_path = write_env_overlay(
            &spec.record.worktree_path,
            &OverlayContext {
                ticket_url: spec.record.url.clone(),
                port_plan: port_plan.clone(),
            },
            &merged_env,
            &StdEnvFs,
        )?;
        let spawn_env = load_merged_env(&spec.record.worktree_path, &overlay_path, &StdEnvFs)?
            .into_iter()
            .collect::<Vec<_>>();

        lifecycle_progress(ui, "starting shared proxy");
        self.ports.ensure_proxy()?;
        lifecycle_progress(ui, "registering proxy route");
        let route = render_route_with_upstream(
            &spec.names,
            &RouteUpstream::HostGateway {
                port: port_plan.app_host_port,
            },
        );
        self.ports.write_route(&spec.record.route_path, &route)?;

        let compose_files = spec.record.compose_files();
        let infra_label = if infra_services.is_empty() {
            "starting infra services".to_owned()
        } else {
            format!("starting infra: {}", infra_services.join(", "))
        };
        lifecycle_progress(ui, &infra_label);
        if let Err(error) =
            self.ports
                .compose_up_infra(&spec.record.project, &compose_files, &infra_services)
        {
            let _ = self.ports.remove_route(&spec.record.route_path);
            return Err(error);
        }

        let script = profile
            .dev_script
            .as_deref()
            .expect("native profile should include a dev script");
        let package_manager = profile.package_manager.unwrap_or(PackageManager::Pnpm);

        if options.detach {
            lifecycle_progress(
                ui,
                &format!(
                    "launching `{script}` in background via {}",
                    install_program(package_manager)
                ),
            );
        } else {
            lifecycle_progress(
                ui,
                &format!(
                    "launching `{script}` in foreground via {}",
                    install_program(package_manager)
                ),
            );
        }

        self.ports.stop_dev_process(&spec.record.worktree_path)?;
        terminate_listeners_on_port(port_plan.app_host_port);

        let launch = NativeDevLaunch {
            worktree_root: spec.record.worktree_path.clone(),
            package_manager,
            script: script.to_owned(),
            env: spawn_env,
        };

        let background_pid = if options.detach {
            let pid = match self.ports.spawn_dev_process(
                &launch.worktree_root,
                launch.package_manager,
                &launch.script,
                &launch.env,
            ) {
                Ok(pid) => pid,
                Err(error) => {
                    let _ = self
                        .ports
                        .compose_down(&spec.record.project, &compose_files, false);
                    let _ = self.ports.remove_route(&spec.record.route_path);
                    return Err(error);
                }
            };
            if let Err(error) =
                ensure_dev_process_running(&StdProcessFs, &launch.worktree_root, pid)
            {
                let _ = self
                    .ports
                    .compose_down(&spec.record.project, &compose_files, false);
                let _ = self.ports.remove_route(&spec.record.route_path);
                return Err(error);
            }
            Some(pid)
        } else {
            None
        };

        spec.record.runtime_mode = Some(RuntimeMode::Native);
        spec.record.dev_script = Some(script.to_owned());
        spec.record.app_host_port = Some(port_plan.app_host_port);
        spec.record.env_overlay_path = Some(overlay_path);
        spec.record.port_plan = Some(port_plan.clone());
        spec.record.status = EnvironmentStatus::Running;

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
            warnings: filter_managed_port_warnings(
                merged_inspection.warnings().to_vec(),
                &port_plan,
            ),
            native_dev: if options.detach { None } else { Some(launch) },
            background_pid,
        })
    }

    fn resolve_project_profile(
        &self,
        _spec: &EnvironmentSpec,
        compose_service_names: &[String],
        script_override: Option<&str>,
    ) -> Result<ProjectProfile> {
        build_project_profile(
            &self.config,
            &self.repo_root,
            compose_service_names,
            script_override,
            &StdDetectFs,
        )
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

    /// Returns a tracked environment by ID.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::EnvironmentNotFound`] when the ID is not tracked.
    pub fn find_record(&self, id: &str) -> Result<EnvironmentRecord> {
        self.list()?
            .into_iter()
            .find(|candidate| candidate.ticket.eq_ignore_ascii_case(id))
            .ok_or_else(|| DinopodError::EnvironmentNotFound {
                ticket: id.to_owned(),
            })
    }

    /// Stops an environment while retaining containers and volumes.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the environment is unknown or Compose fails.
    pub fn stop(&self, ticket: &str, mut ui: Option<&mut dyn Ui>) -> Result<()> {
        lifecycle_progress(&mut ui, "stopping pod");
        let result = self.update_record(ticket, |record| {
            if record.runtime_mode == Some(RuntimeMode::Native) {
                lifecycle_progress(&mut ui, "stopping native dev process");
                self.ports.stop_dev_process(&record.worktree_path)?;
            }
            lifecycle_progress(&mut ui, "stopping compose project");
            self.ports
                .compose_stop(&record.project, &record.compose_files())?;
            record.status = EnvironmentStatus::Stopped;
            Ok(())
        });
        match &result {
            Ok(()) => lifecycle_finalize(&mut ui),
            Err(_) => lifecycle_fail(&mut ui),
        }
        result
    }

    /// Downs an environment and removes its route.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when the environment is unknown, Compose fails,
    /// or the route cannot be removed.
    pub fn down(&self, ticket: &str, volumes: bool, mut ui: Option<&mut dyn Ui>) -> Result<()> {
        let label = if volumes {
            "tearing down pod and volumes"
        } else {
            "tearing down pod"
        };
        lifecycle_progress(&mut ui, label);
        let result = self.update_record(ticket, |record| {
            if record.runtime_mode == Some(RuntimeMode::Native) {
                lifecycle_progress(&mut ui, "stopping native dev process");
                self.ports.stop_dev_process(&record.worktree_path)?;
            }
            lifecycle_progress(&mut ui, "tearing down compose project");
            self.ports
                .compose_down(&record.project, &record.compose_files(), volumes)?;
            lifecycle_progress(&mut ui, "removing proxy route");
            self.ports.remove_route(&record.route_path)?;
            record.status = EnvironmentStatus::Down;
            Ok(())
        });
        match &result {
            Ok(()) => lifecycle_finalize(&mut ui),
            Err(_) => lifecycle_fail(&mut ui),
        }
        result
    }

    /// Removes an environment and its worktree after confirmation.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::ConfirmationRequired`] when `confirmed` is false.
    pub fn rm(&self, ticket: &str, confirmed: bool, mut ui: Option<&mut dyn Ui>) -> Result<()> {
        if !confirmed {
            return Err(DinopodError::ConfirmationRequired {
                ticket: ticket.to_owned(),
            });
        }

        lifecycle_progress(&mut ui, "planning removal");
        let result = self.rm_steps(ticket, &mut ui);
        match &result {
            Ok(()) => lifecycle_finalize(&mut ui),
            Err(_) => lifecycle_fail(&mut ui),
        }
        result
    }

    fn rm_steps(&self, ticket: &str, ui: &mut Option<&mut dyn Ui>) -> Result<()> {
        let project = self.project_for_ticket(ticket)?;
        let mut records = self.state.load()?;
        let Some(record) = records.remove(&project) else {
            return Err(DinopodError::EnvironmentNotFound {
                ticket: ticket.to_owned(),
            });
        };

        if record.runtime_mode == Some(RuntimeMode::Native) {
            lifecycle_progress(ui, "stopping native dev process");
            self.ports.stop_dev_process(&record.worktree_path)?;
        }
        lifecycle_progress(ui, "tearing down compose project");
        self.ports
            .compose_down(&record.project, &record.compose_files(), false)?;
        lifecycle_progress(ui, "removing proxy route");
        self.ports.remove_route(&record.route_path)?;
        lifecycle_progress(ui, "removing git worktree");
        self.ports
            .remove_worktree(&self.repo_root, &record.worktree_path)?;
        lifecycle_progress(ui, "removing pod state");
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
        let user_compose_path = worktree_path.join(self.config.compose_file());

        Ok(EnvironmentSpec {
            record: EnvironmentRecord {
                project,
                ticket: ticket.to_owned(),
                host,
                url,
                worktree_path,
                route_path,
                user_compose_path: Some(user_compose_path.clone()),
                compose_override_path: Some(compose_override_path.clone()),
                status: EnvironmentStatus::Running,
                runtime_mode: None,
                dev_script: None,
                app_host_port: None,
                env_overlay_path: None,
                port_plan: None,
            },
            user_compose_path,
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

    /// Returns merged dotenv + overlay variables for a worktree.
    ///
    /// # Errors
    ///
    /// Returns a recoverable Dinopod error when env files cannot be read.
    pub fn merged_env_for_worktree(&self, worktree_root: &Path) -> Result<Vec<(String, String)>> {
        let overlay_path = worktree_root.join(".dinopod/env.overlay");
        Ok(load_merged_env(worktree_root, &overlay_path, &StdEnvFs)?
            .into_iter()
            .collect())
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
    user_compose_path: PathBuf,
    compose_override_path: PathBuf,
    names: EnvironmentNames,
}
