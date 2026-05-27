use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::Path;
use std::process;

use dinopod::cmd::{CommandOutput, CommandRunner, CommandSpec};
use dinopod::config::DinopodConfig;
use dinopod::errors::DinopodError;
use dinopod::lifecycle::LifecyclePorts;
use dinopod::proxy::ProxyPaths;
use dinopod::runtime::CommandLifecyclePorts;

#[derive(Debug)]
struct RecordingRunner {
    commands: RefCell<Vec<CommandSpec>>,
    outputs: RefCell<VecDeque<CommandOutput>>,
}

impl RecordingRunner {
    fn new(outputs: Vec<CommandOutput>) -> Self {
        Self {
            commands: RefCell::new(Vec::new()),
            outputs: RefCell::new(outputs.into()),
        }
    }

    fn commands(&self) -> Vec<CommandSpec> {
        self.commands.borrow().clone()
    }
}

impl Default for RecordingRunner {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl CommandRunner for &RecordingRunner {
    fn run(&self, command: &CommandSpec) -> io::Result<CommandOutput> {
        self.commands.borrow_mut().push(command.clone());
        Ok(self
            .outputs
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| CommandOutput::successful("", "")))
    }
}

fn proxy_inspect_output(image: &str, port: u16, dynamic_dir: &Path) -> String {
    format!(
        "true\t{image}\t{{\"{port}/tcp\":[{{\"HostPort\":\"{port}\"}}]}}\t[{{\"Source\":\"{}\",\"Destination\":\"/etc/traefik/dynamic\"}}]",
        dynamic_dir.display()
    )
}

#[test]
fn ensure_proxy_should_create_dynamic_config_directory_before_starting_proxy() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-runtime-dynamic-dir-test-{}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    let proxy_paths = ProxyPaths::new(&temp_dir);
    let dynamic_dir = proxy_paths.resolved_dynamic_config_dir();
    let runner = RecordingRunner::new(vec![
        CommandOutput::successful(proxy_inspect_output("traefik:v3.6", 80, &dynamic_dir), ""),
        CommandOutput::successful("", ""),
        CommandOutput::successful("", ""),
    ]);
    let ports = CommandLifecyclePorts::new(&runner, DinopodConfig::default(), proxy_paths);

    ports.ensure_proxy().expect("proxy should be ensured");

    assert!(dynamic_dir.is_dir());
}

#[test]
fn ensure_proxy_should_write_proxy_compose_before_starting_proxy() {
    let temp_dir =
        std::env::temp_dir().join(format!("dinopod-runtime-proxy-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let config = DinopodConfig::default();
    let proxy_paths = ProxyPaths::new(&temp_dir);
    let runner = RecordingRunner::default();
    let ports = CommandLifecyclePorts::new(&runner, config, proxy_paths);

    ports.ensure_proxy().expect("proxy should be ensured");

    let compose = fs::read_to_string(temp_dir.join("proxy").join("compose.yaml"))
        .expect("proxy compose file should be written");
    assert!(compose.contains("image: traefik:v3.6"));
    assert!(!compose.contains("/var/run/docker.sock"));
}

#[test]
fn ensure_proxy_should_reuse_running_proxy_with_expected_image() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-runtime-healthy-proxy-test-{}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let proxy_paths = ProxyPaths::new(&temp_dir);
    let dynamic_dir = proxy_paths.resolved_dynamic_config_dir();
    let runner = RecordingRunner::new(vec![CommandOutput::successful(
        proxy_inspect_output("traefik:v3.6", 80, &dynamic_dir),
        "",
    )]);
    let ports = CommandLifecyclePorts::new(&runner, DinopodConfig::default(), proxy_paths);

    ports
        .ensure_proxy()
        .expect("healthy proxy should be reused");

    let commands = runner.commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].arguments(),
        [
            "inspect",
            "dinopod-traefik",
            "--format",
            dinopod::proxy::PROXY_INSPECT_FORMAT,
        ]
    );
}

#[test]
fn ensure_proxy_should_repair_running_proxy_with_stale_port_binding() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-runtime-stale-port-proxy-test-{}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let proxy_paths = ProxyPaths::new(&temp_dir);
    let dynamic_dir = proxy_paths.resolved_dynamic_config_dir();
    let runner = RecordingRunner::new(vec![
        CommandOutput::successful(proxy_inspect_output("traefik:v3.6", 8080, &dynamic_dir), ""),
        CommandOutput::successful("", ""),
        CommandOutput::successful("", ""),
    ]);
    let ports = CommandLifecyclePorts::new(&runner, DinopodConfig::default(), proxy_paths);

    ports
        .ensure_proxy()
        .expect("stale port binding should trigger repair");

    let commands = runner.commands();
    assert_eq!(commands[1].arguments(), ["rm", "-f", "dinopod-traefik"]);
}

#[test]
fn ensure_proxy_should_repair_running_proxy_with_wrong_image() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-runtime-repair-proxy-test-{}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let proxy_paths = ProxyPaths::new(&temp_dir);
    let dynamic_dir = proxy_paths.resolved_dynamic_config_dir();
    let runner = RecordingRunner::new(vec![
        CommandOutput::successful(proxy_inspect_output("traefik:v2.11", 80, &dynamic_dir), ""),
        CommandOutput::successful("", ""),
        CommandOutput::successful("", ""),
    ]);
    let ports = CommandLifecyclePorts::new(&runner, DinopodConfig::default(), proxy_paths);

    ports
        .ensure_proxy()
        .expect("wrong-image proxy should be repaired");

    let commands = runner.commands();
    assert_eq!(commands[1].arguments(), ["rm", "-f", "dinopod-traefik"]);
}

#[test]
fn compose_up_should_inspect_app_service_before_starting_project() {
    let temp_dir =
        std::env::temp_dir().join(format!("dinopod-runtime-compose-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let compose_file = temp_dir.join("compose.yaml");
    let override_file = temp_dir.join("compose.override.yaml");
    fs::write(&compose_file, "services: {}\n").expect("compose file should be written");
    fs::write(&override_file, "services: {}\n").expect("override file should be written");
    let runner = RecordingRunner::new(vec![CommandOutput::successful(
        r#"{"services":{"worker":{}}}"#,
        "",
    )]);
    let ports = CommandLifecyclePorts::new(
        &runner,
        DinopodConfig::default(),
        ProxyPaths::new(temp_dir.join("config")),
    );
    let compose_files = vec![compose_file.clone(), override_file];

    let error = ports
        .compose_up("project", &compose_files)
        .expect_err("missing app service should fail before compose up");

    assert!(matches!(
        error,
        DinopodError::ComposeServiceMissing { service } if service == "app"
    ));
    let commands = runner.commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].arguments(),
        [
            "compose",
            "-f",
            &compose_file.display().to_string(),
            "config",
            "--format",
            "json"
        ]
    );
}

#[test]
fn compose_up_should_start_whole_compose_project() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-runtime-compose-service-test-{}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let compose_file = temp_dir.join("compose.yaml");
    let override_file = temp_dir.join("compose.override.yaml");
    fs::write(&compose_file, "services: {}\n").expect("compose file should be written");
    fs::write(&override_file, "services: {}\n").expect("override file should be written");
    let runner = RecordingRunner::new(vec![
        CommandOutput::successful(r#"{"services":{"app":{},"app_with_fixed_port":{}}}"#, ""),
        CommandOutput::successful("", ""),
    ]);
    let ports = CommandLifecyclePorts::new(
        &runner,
        DinopodConfig::default(),
        ProxyPaths::new(temp_dir.join("config")),
    );

    ports
        .compose_up("project", &[compose_file, override_file])
        .expect("compose up should run");

    let commands = runner.commands();
    assert_eq!(
        commands[1].arguments(),
        [
            "compose",
            "-p",
            "project",
            "-f",
            &commands[1].arguments()[4],
            "-f",
            &commands[1].arguments()[6],
            "up",
            "-d",
        ]
    );
}

#[test]
fn write_compose_override_should_create_dinopod_directory_before_writing() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-runtime-compose-override-test-{}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let override_path = temp_dir.join(".dinopod").join("compose.override.yml");
    let runner = RecordingRunner::default();
    let ports = CommandLifecyclePorts::new(
        &runner,
        DinopodConfig::default(),
        ProxyPaths::new(temp_dir.join("config")),
    );

    ports
        .write_compose_override(&override_path, "services: {}\n")
        .expect("compose override should be written");

    assert!(override_path.is_file());
    let _ = fs::remove_dir_all(&temp_dir);
}
