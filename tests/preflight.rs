use std::collections::HashSet;
use std::path::Path;

use dinopod::errors::DinopodError;
use dinopod::preflight::{
    Dependency, PortStatus, PreflightChecker, PreflightProbe, ProxyPortStatus,
};

#[derive(Debug, Default)]
struct FakeProbe {
    commands: HashSet<&'static str>,
    docker_daemon_available: bool,
    docker_compose_available: bool,
    inside_git_repo: bool,
    proxy_port_status: ProxyPortStatus,
}

impl PreflightProbe for FakeProbe {
    fn command_exists(&self, command: &str) -> bool {
        self.commands.contains(command)
    }

    fn docker_daemon_available(&self) -> bool {
        self.docker_daemon_available
    }

    fn docker_compose_available(&self) -> bool {
        self.docker_compose_available
    }

    fn inside_git_repo(&self, _path: &Path) -> bool {
        self.inside_git_repo
    }

    fn proxy_port_status(&self, _port: u16, _container_name: &str) -> ProxyPortStatus {
        self.proxy_port_status
    }
}

#[test]
fn dependency_check_should_report_missing_git() {
    let checker = PreflightChecker::new(FakeProbe::default());
    let error = checker
        .require_command(Dependency::Git)
        .expect_err("missing git should fail");

    assert!(matches!(
        error,
        DinopodError::MissingDependency(Dependency::Git)
    ));
}

#[test]
fn dependency_check_should_report_missing_docker_compose() {
    let mut probe = FakeProbe::default();
    probe.commands.insert("docker");
    let checker = PreflightChecker::new(probe);
    let error = checker
        .require_command(Dependency::DockerCompose)
        .expect_err("missing docker compose should fail");

    assert!(matches!(
        error,
        DinopodError::MissingDependency(Dependency::DockerCompose)
    ));
}

#[test]
fn docker_daemon_check_should_distinguish_missing_binary_from_stopped_daemon() {
    let mut probe = FakeProbe::default();
    probe.commands.insert("docker");
    let checker = PreflightChecker::new(probe);
    let error = checker
        .require_docker_daemon()
        .expect_err("stopped daemon should fail");

    assert!(matches!(error, DinopodError::DockerDaemonUnavailable));
}

#[test]
fn proxy_port_check_should_reuse_existing_healthy_dinopod_proxy() {
    let checker = PreflightChecker::new(FakeProbe {
        proxy_port_status: ProxyPortStatus::InUseByDinopod,
        ..FakeProbe::default()
    });

    assert_eq!(
        checker
            .check_proxy_port(80, "dinopod-traefik")
            .expect("healthy Dinopod proxy should be reusable"),
        PortStatus::ReusableDinopodProxy
    );
}

#[test]
fn proxy_port_check_should_reject_other_process_on_proxy_port() {
    let checker = PreflightChecker::new(FakeProbe {
        proxy_port_status: ProxyPortStatus::InUseByOtherProcess,
        ..FakeProbe::default()
    });
    let error = checker
        .check_proxy_port(80, "dinopod-traefik")
        .expect_err("other process should block proxy startup");

    assert!(matches!(error, DinopodError::PortInUse { port: 80 }));
}
