//! Production adapters that connect lifecycle orchestration to local commands and files.

use std::path::{Path, PathBuf};

use crate::cmd::{
    docker_command_failed, git_command_failed, path_display, CommandRunner, CommandSpec,
    StdCommandRunner,
};
use crate::compose::{
    build_compose_command, build_compose_infra_up_command, inspect_compose_config_for_runtime,
    ComposeFiles, ComposeInspection, ComposeValidator, StdComposeFs,
};
use crate::config::{DinopodConfig, RuntimeMode};
use crate::detect::PackageManager;
use crate::env::{
    copy_env_files_on_create, install_arguments, install_program, refresh_env_files,
    sync_missing_env_files, StdEnvFs,
};
use crate::errors::{DinopodError, Result};
use crate::fs::{AtomicFileSystem, AtomicWriter, StdAtomicFileSystem};
use crate::git::{GitWorktreeManager, StdWorktreeFs, WorktreeAction, WorktreeRequest};
use crate::lifecycle::LifecyclePorts;
use crate::process::{spawn_dev_process, stop_dev_process, StdProcessFs};
use crate::proxy::{
    classify_proxy_container, render_proxy_compose, ProxyManager, ProxyPaths, ProxyRuntimeSpec,
    ProxyStatus, PROXY_INSPECT_FORMAT,
};

/// Lifecycle ports backed by local Git, Docker, and filesystem operations.
#[derive(Clone, Debug)]
pub struct CommandLifecyclePorts<R = StdCommandRunner> {
    runner: R,
    config: DinopodConfig,
    proxy_paths: ProxyPaths,
}

impl<R> CommandLifecyclePorts<R>
where
    R: CommandRunner,
{
    /// Creates command-backed lifecycle ports.
    #[must_use]
    pub fn new(runner: R, config: DinopodConfig, proxy_paths: ProxyPaths) -> Self {
        Self {
            runner,
            config,
            proxy_paths,
        }
    }

    fn run_git<I, S>(&self, current_dir: &Path, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let output = self.runner.run(
            &CommandSpec::new("git")
                .args(args.clone())
                .current_dir(current_dir.to_path_buf()),
        )?;
        if output.success() {
            Ok(())
        } else {
            Err(git_command_failed(args, &output))
        }
    }
}

impl<R> LifecyclePorts for CommandLifecyclePorts<R>
where
    R: CommandRunner,
{
    fn ensure_worktree(
        &self,
        repo_root: &Path,
        worktree_path: &Path,
        branch: &str,
        default_branch: &str,
    ) -> Result<WorktreeAction> {
        let request = WorktreeRequest::new(repo_root, worktree_path, branch, default_branch);
        GitWorktreeManager::new(&self.runner, StdWorktreeFs).ensure_worktree(&request)
    }

    fn write_compose_override(&self, path: &Path, contents: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut writer = AtomicWriter::new(StdAtomicFileSystem);
        writer.write_atomic(path, contents)?;
        Ok(())
    }

    fn write_route(&self, path: &Path, contents: &str) -> Result<()> {
        let mut writer = AtomicWriter::new(StdAtomicFileSystem);
        writer.write_atomic(path, contents)?;
        Ok(())
    }

    fn remove_route(&self, path: &Path) -> Result<()> {
        let mut fs = StdAtomicFileSystem;
        fs.remove_file(path)?;
        Ok(())
    }

    fn inspect_user_compose(&self, user_file: &Path) -> Result<ComposeInspection> {
        self.inspect_user_compose_file(user_file)
    }

    fn ensure_proxy(&self) -> Result<()> {
        std::fs::create_dir_all(self.proxy_paths.dynamic_config_dir())?;
        let mut writer = AtomicWriter::new(StdAtomicFileSystem);
        writer.write_atomic(
            self.proxy_paths.compose_file(),
            &render_proxy_compose(&self.config, &self.proxy_paths),
        )?;
        let status = self.inspect_proxy_status()?;
        ProxyManager::new(&self.runner).ensure_proxy(&self.config, &self.proxy_paths, status)?;
        Ok(())
    }

    fn compose_up(&self, project: &str, compose_files: &[PathBuf]) -> Result<ComposeInspection> {
        let Some((user_file, rest)) = compose_files.split_first() else {
            return Ok(ComposeInspection::default());
        };
        let Some(override_file) = rest.first() else {
            return Ok(ComposeInspection::default());
        };
        let inspection = self.inspect_user_compose_file(user_file)?;
        if !inspection
            .service_names()
            .iter()
            .any(|service| service == &self.config.app.service)
        {
            return Err(DinopodError::ComposeServiceMissing {
                service: self.config.app.service.clone(),
            });
        }
        let files = crate::compose::ComposeFiles::new(user_file, override_file);
        let command = build_compose_command(project, &files, compose_up_args());
        let output = self.runner.run(&command)?;
        if output.success() {
            Ok(inspection)
        } else {
            Err(docker_command_failed(command.arguments().to_vec(), &output))
        }
    }

    fn compose_stop(&self, project: &str, compose_files: &[PathBuf]) -> Result<()> {
        self.run_compose(project, compose_files, ["stop"])
    }

    fn compose_down(&self, project: &str, compose_files: &[PathBuf], volumes: bool) -> Result<()> {
        if volumes {
            self.run_compose(project, compose_files, ["down", "--volumes"])
        } else {
            self.run_compose(project, compose_files, ["down"])
        }
    }

    fn remove_worktree(&self, repo_root: &Path, path: &Path) -> Result<()> {
        self.run_git(
            repo_root,
            vec![
                "worktree".to_owned(),
                "remove".to_owned(),
                "--force".to_owned(),
                path_display(path),
            ],
        )
    }

    fn project_is_running(&self, project: &str) -> Result<bool> {
        let args = ["compose", "-p", project, "ps", "-q"]
            .map(ToOwned::to_owned)
            .to_vec();
        let output = self
            .runner
            .run(&CommandSpec::new("docker").args(args.clone()))?;
        if output.success() {
            Ok(!output.stdout().trim().is_empty())
        } else {
            Err(docker_command_failed(args, &output))
        }
    }

    fn copy_env_on_create(&self, source_root: &Path, worktree_root: &Path) -> Result<()> {
        copy_env_files_on_create(source_root, worktree_root, &self.config.settings, &StdEnvFs)
    }

    fn sync_missing_env(&self, source_root: &Path, worktree_root: &Path) -> Result<()> {
        sync_missing_env_files(source_root, worktree_root, &self.config.settings, &StdEnvFs)
    }

    fn refresh_env(&self, source_root: &Path, worktree_root: &Path) -> Result<()> {
        refresh_env_files(source_root, worktree_root, &StdEnvFs)
    }

    fn install_dependencies(
        &self,
        worktree_root: &Path,
        package_manager: PackageManager,
    ) -> Result<()> {
        let command = CommandSpec::new(install_program(package_manager))
            .args(
                install_arguments(package_manager)
                    .iter()
                    .copied()
                    .map(str::to_owned),
            )
            .current_dir(worktree_root.to_path_buf());
        let output = self.runner.run(&command)?;
        if output.success() {
            Ok(())
        } else {
            Err(docker_command_failed(command.arguments().to_vec(), &output))
        }
    }

    fn inspect_user_compose_merged(&self, user_file: &Path) -> Result<(ComposeInspection, String)> {
        self.inspect_compose_json(user_file, None, RuntimeMode::Native)
    }

    fn inspect_compose_stack(
        &self,
        user_file: &Path,
        dinopod_override: &Path,
    ) -> Result<(ComposeInspection, String)> {
        self.inspect_compose_json(user_file, Some(dinopod_override), RuntimeMode::Native)
    }

    fn compose_up_all(
        &self,
        project: &str,
        compose_files: &[PathBuf],
    ) -> Result<ComposeInspection> {
        let Some((user_file, rest)) = compose_files.split_first() else {
            return Ok(ComposeInspection::default());
        };
        let Some(override_file) = rest.first() else {
            return Ok(ComposeInspection::default());
        };
        let (_, compose_json) =
            self.inspect_compose_json(user_file, Some(override_file), RuntimeMode::Native)?;
        let inspection = crate::compose::inspect_compose_services(&compose_json)?;
        let files = ComposeFiles::new(user_file, override_file);
        let command = build_compose_command(project, &files, compose_up_args());
        let output = self.runner.run(&command)?;
        if output.success() {
            Ok(inspection)
        } else {
            Err(docker_command_failed(command.arguments().to_vec(), &output))
        }
    }

    fn run_setup_command(
        &self,
        worktree_root: &Path,
        command: &str,
        env: &[(String, String)],
    ) -> Result<()> {
        let mut spec = CommandSpec::new("sh")
            .args(["-c", command])
            .current_dir(worktree_root);
        for (key, value) in env {
            spec = spec.env(key, value);
        }
        let output = self.runner.run(&spec)?;
        if output.success() {
            Ok(())
        } else {
            Err(DinopodError::SetupCommandFailed {
                command: command.to_owned(),
                stderr: output.stderr().to_owned(),
            })
        }
    }

    fn compose_up_infra(
        &self,
        project: &str,
        compose_files: &[PathBuf],
        services: &[String],
    ) -> Result<ComposeInspection> {
        let Some((user_file, rest)) = compose_files.split_first() else {
            return Ok(ComposeInspection::default());
        };
        let Some(override_file) = rest.first() else {
            return Ok(ComposeInspection::default());
        };
        let inspection = self.inspect_user_compose_file(user_file)?;
        if services.is_empty() {
            return Ok(inspection);
        }
        let files = ComposeFiles::new(user_file, override_file);
        let command = build_compose_infra_up_command(project, &files, services.iter().cloned());
        let output = self.runner.run(&command)?;
        if output.success() {
            Ok(inspection)
        } else {
            Err(docker_command_failed(command.arguments().to_vec(), &output))
        }
    }

    fn spawn_dev_process(
        &self,
        worktree_root: &Path,
        package_manager: PackageManager,
        script: &str,
        env: &[(String, String)],
    ) -> Result<u32> {
        spawn_dev_process(
            &self.runner,
            &StdProcessFs,
            worktree_root,
            package_manager,
            script,
            env,
        )
    }

    fn stop_dev_process(&self, worktree_root: &Path) -> Result<()> {
        stop_dev_process(&StdProcessFs, worktree_root)
    }
}

impl<R> CommandLifecyclePorts<R>
where
    R: CommandRunner,
{
    fn run_compose<I, S>(&self, project: &str, compose_files: &[PathBuf], args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let Some((user_file, rest)) = compose_files.split_first() else {
            return Ok(());
        };
        let Some(override_file) = rest.first() else {
            return Ok(());
        };
        let files = crate::compose::ComposeFiles::new(user_file, override_file);
        let command = build_compose_command(project, &files, args);
        let output = self.runner.run(&command)?;
        if output.success() {
            Ok(())
        } else {
            Err(docker_command_failed(command.arguments().to_vec(), &output))
        }
    }

    fn inspect_user_compose_file(&self, user_file: &Path) -> Result<ComposeInspection> {
        let (inspection, _) = self.inspect_compose_json(user_file, None, RuntimeMode::Native)?;
        Ok(inspection)
    }

    fn inspect_compose_json(
        &self,
        user_file: &Path,
        dinopod_override: Option<&Path>,
        runtime: RuntimeMode,
    ) -> Result<(ComposeInspection, String)> {
        ComposeValidator::new(StdComposeFs).require_compose_file(user_file)?;
        let user_override = user_file.with_file_name("docker-compose.override.yml");
        let mut inspect_args = vec![
            "compose".to_owned(),
            "-f".to_owned(),
            user_file.display().to_string(),
        ];
        if user_override.is_file() {
            inspect_args.push("-f".to_owned());
            inspect_args.push(user_override.display().to_string());
        }
        if let Some(override_file) = dinopod_override {
            inspect_args.push("-f".to_owned());
            inspect_args.push(override_file.display().to_string());
        }
        inspect_args.extend([
            "config".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ]);
        let inspect_output = self
            .runner
            .run(&CommandSpec::new("docker").args(inspect_args.clone()))?;
        if !inspect_output.success() {
            return Err(docker_command_failed(inspect_args, &inspect_output));
        }
        let json = inspect_output.stdout().to_owned();
        let inspection =
            inspect_compose_config_for_runtime(&json, &self.config.app.service, runtime)?;
        Ok((inspection, json))
    }

    fn inspect_proxy_status(&self) -> Result<ProxyStatus> {
        let args = vec![
            "inspect".to_owned(),
            self.config.proxy.container_name.clone(),
            "--format".to_owned(),
            PROXY_INSPECT_FORMAT.to_owned(),
        ];
        let output = self.runner.run(&CommandSpec::new("docker").args(args))?;
        if !output.success() {
            return Ok(ProxyStatus::Stopped);
        }

        Ok(classify_proxy_container(
            output.stdout().trim_end(),
            &ProxyRuntimeSpec::from_config(&self.config, &self.proxy_paths),
        ))
    }
}

fn compose_up_args() -> Vec<String> {
    vec!["up".to_owned(), "-d".to_owned()]
}
