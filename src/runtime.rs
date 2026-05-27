//! Production adapters that connect lifecycle orchestration to local commands and files.

use std::path::{Path, PathBuf};

use crate::cmd::{
    docker_command_failed, git_command_failed, path_display, CommandRunner, CommandSpec,
    StdCommandRunner,
};
use crate::compose::{
    build_compose_command, inspect_compose_config, ComposeInspection, ComposeValidator,
    StdComposeFs,
};
use crate::config::DinopodConfig;
use crate::errors::Result;
use crate::fs::{AtomicFileSystem, AtomicWriter, StdAtomicFileSystem};
use crate::git::{GitWorktreeManager, StdWorktreeFs, WorktreeRequest};
use crate::lifecycle::LifecyclePorts;
use crate::proxy::{render_proxy_compose, ProxyManager, ProxyPaths, ProxyStatus};

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
    ) -> Result<()> {
        let request = WorktreeRequest::new(repo_root, worktree_path, branch, default_branch);
        GitWorktreeManager::new(&self.runner, StdWorktreeFs).ensure_worktree(&request)?;
        Ok(())
    }

    fn write_compose_override(&self, path: &Path, contents: &str) -> Result<()> {
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

    fn ensure_proxy(&self) -> Result<()> {
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
        ComposeValidator::new(StdComposeFs).require_compose_file(user_file)?;
        let inspect_args = vec![
            "compose".to_owned(),
            "-f".to_owned(),
            user_file.display().to_string(),
            "config".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ];
        let inspect_output = self
            .runner
            .run(&CommandSpec::new("docker").args(inspect_args.clone()))?;
        if !inspect_output.success() {
            return Err(docker_command_failed(inspect_args, &inspect_output));
        }
        let inspection = inspect_compose_config(inspect_output.stdout(), &self.config.app.service)?;
        let files = crate::compose::ComposeFiles::new(user_file, override_file);
        let command =
            build_compose_command(project, &files, compose_up_args(&self.config.app.service));
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

    fn inspect_proxy_status(&self) -> Result<ProxyStatus> {
        let args = vec![
            "inspect".to_owned(),
            self.config.proxy.container_name.clone(),
            "--format".to_owned(),
            "{{.State.Running}} {{.Config.Image}}".to_owned(),
        ];
        let output = self.runner.run(&CommandSpec::new("docker").args(args))?;
        if !output.success() {
            return Ok(ProxyStatus::Stopped);
        }

        let mut fields = output.stdout().split_whitespace();
        let running = fields.next();
        let image = fields.next();
        match (running, image) {
            (Some("true"), Some(image)) if image == self.config.proxy.image => {
                Ok(ProxyStatus::Healthy)
            }
            (Some("true"), Some(_)) => Ok(ProxyStatus::NeedsRepair),
            _ => Ok(ProxyStatus::Stopped),
        }
    }
}

fn compose_up_args(service: &str) -> Vec<String> {
    vec!["up".to_owned(), "-d".to_owned(), service.to_owned()]
}
