//! Preflight checks for external machine dependencies.

use std::fmt;
use std::net::TcpListener;
use std::path::Path;

use crate::cmd::{CommandRunner, CommandSpec, StdCommandRunner};
use crate::errors::{DinopodError, Result};

/// External dependency required by Dinopod.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dependency {
    /// Git command-line tool.
    Git,
    /// Docker command-line tool.
    Docker,
    /// Docker Compose plugin.
    DockerCompose,
}

impl Dependency {
    /// Returns the shell command used to check this dependency.
    #[must_use]
    pub fn command(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Docker | Self::DockerCompose => "docker",
        }
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Git => "git",
            Self::Docker => "docker",
            Self::DockerCompose => "docker compose",
        })
    }
}

/// Ownership status for the configured proxy port.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProxyPortStatus {
    /// The port is currently free.
    #[default]
    Free,
    /// The port is in use by a healthy Dinopod proxy.
    InUseByDinopod,
    /// The port is in use by another process.
    InUseByOtherProcess,
}

/// Result of checking the configured proxy port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortStatus {
    /// The port is free for proxy startup.
    Available,
    /// A healthy existing Dinopod proxy can be reused.
    ReusableDinopodProxy,
}

/// System probes used by preflight checks.
pub trait PreflightProbe {
    /// Returns true when a command is available.
    fn command_exists(&self, command: &str) -> bool;

    /// Returns true when the Docker daemon is available.
    fn docker_daemon_available(&self) -> bool;

    /// Returns true when `docker compose` is available.
    fn docker_compose_available(&self) -> bool;

    /// Returns true when `path` is inside a Git repository.
    fn inside_git_repo(&self, path: &Path) -> bool;

    /// Returns ownership information for the configured proxy port.
    fn proxy_port_status(&self, port: u16, container_name: &str) -> ProxyPortStatus;
}

/// Runs preflight checks using a supplied probe implementation.
#[derive(Debug)]
pub struct PreflightChecker<P> {
    probe: P,
}

/// Production preflight probe backed by local commands and TCP bind checks.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandPreflightProbe<R = StdCommandRunner> {
    runner: R,
}

impl<R> CommandPreflightProbe<R>
where
    R: CommandRunner,
{
    /// Creates a command-backed preflight probe.
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    fn command_success<I, S>(&self, program: &str, args: I, current_dir: Option<&Path>) -> bool
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut command = CommandSpec::new(program).args(args);
        if let Some(current_dir) = current_dir {
            command = command.current_dir(current_dir.to_path_buf());
        }

        self.runner
            .run(&command)
            .is_ok_and(|output| output.success())
    }
}

impl<R> PreflightProbe for CommandPreflightProbe<R>
where
    R: CommandRunner,
{
    fn command_exists(&self, command: &str) -> bool {
        self.command_success(command, ["--version"], None)
    }

    fn docker_daemon_available(&self) -> bool {
        self.command_success("docker", ["info"], None)
    }

    fn docker_compose_available(&self) -> bool {
        self.command_success("docker", ["compose", "version"], None)
    }

    fn inside_git_repo(&self, path: &Path) -> bool {
        let command = CommandSpec::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(path.to_path_buf());
        self.runner
            .run(&command)
            .is_ok_and(|output| output.success() && output.stdout().trim() == "true")
    }

    fn proxy_port_status(&self, port: u16, container_name: &str) -> ProxyPortStatus {
        let name_filter = format!("name=^/{container_name}$");
        let output = self.runner.run(&CommandSpec::new("docker").args(vec![
            "ps".to_owned(),
            "--filter".to_owned(),
            name_filter,
            "--filter".to_owned(),
            "status=running".to_owned(),
            "--format".to_owned(),
            "{{.Ports}}".to_owned(),
        ]));

        if output.is_ok_and(|output| output.success() && ports_include(output.stdout(), port)) {
            return ProxyPortStatus::InUseByDinopod;
        }

        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            ProxyPortStatus::Free
        } else {
            ProxyPortStatus::InUseByOtherProcess
        }
    }
}

fn ports_include(ports: &str, port: u16) -> bool {
    ports.contains(&format!(":{port}->")) || ports.contains(&format!("0.0.0.0:{port}"))
}

impl<P> PreflightChecker<P>
where
    P: PreflightProbe,
{
    /// Creates a preflight checker.
    #[must_use]
    pub fn new(probe: P) -> Self {
        Self { probe }
    }

    /// Requires a command-line dependency.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::MissingDependency`] if the dependency is absent.
    pub fn require_command(&self, dependency: Dependency) -> Result<()> {
        if self.dependency_available(dependency) {
            Ok(())
        } else {
            Err(DinopodError::MissingDependency(dependency))
        }
    }

    /// Requires Docker and a running Docker daemon.
    ///
    /// # Errors
    ///
    /// Returns a dependency error if Docker is missing, or
    /// [`DinopodError::DockerDaemonUnavailable`] if Docker is installed but stopped.
    pub fn require_docker_daemon(&self) -> Result<()> {
        self.require_command(Dependency::Docker)?;
        if self.probe.docker_daemon_available() {
            Ok(())
        } else {
            Err(DinopodError::DockerDaemonUnavailable)
        }
    }

    /// Requires Docker Compose.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::MissingDependency`] if Docker is absent or Compose is unavailable.
    pub fn require_docker_compose(&self) -> Result<()> {
        self.require_command(Dependency::Docker)?;
        self.require_command(Dependency::DockerCompose)
    }

    /// Requires the current directory to be inside a Git repository.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::NotInGitRepository`] when the path is not in a repo.
    pub fn require_git_repo(&self, path: &Path) -> Result<()> {
        if self.probe.inside_git_repo(path) {
            Ok(())
        } else {
            Err(DinopodError::NotInGitRepository)
        }
    }

    /// Checks whether the proxy port is usable.
    ///
    /// # Errors
    ///
    /// Returns [`DinopodError::PortInUse`] when another process owns the port.
    pub fn check_proxy_port(&self, port: u16, container_name: &str) -> Result<PortStatus> {
        match self.probe.proxy_port_status(port, container_name) {
            ProxyPortStatus::Free => Ok(PortStatus::Available),
            ProxyPortStatus::InUseByDinopod => Ok(PortStatus::ReusableDinopodProxy),
            ProxyPortStatus::InUseByOtherProcess => Err(DinopodError::PortInUse { port }),
        }
    }

    fn dependency_available(&self, dependency: Dependency) -> bool {
        match dependency {
            Dependency::Git | Dependency::Docker => self.probe.command_exists(dependency.command()),
            Dependency::DockerCompose => {
                self.probe.command_exists(dependency.command())
                    && self.probe.docker_compose_available()
            }
        }
    }
}
