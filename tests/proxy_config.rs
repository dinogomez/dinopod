use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use dinopod::cmd::{CommandOutput, CommandRunner, CommandSpec};
use dinopod::config::DinopodConfig;
use dinopod::fs::{AtomicFileSystem, AtomicWriter};
use dinopod::lock::MutationGuard;
use dinopod::names::derive_names;
use dinopod::proxy::{
    classify_proxy_container, render_proxy_compose, ProxyAction, ProxyManager, ProxyPaths,
    ProxyRuntimeSpec, ProxyStatus,
};
use dinopod::routes::render_route;

#[derive(Debug, Default)]
struct FakeRunner {
    commands: RefCell<Vec<CommandSpec>>,
    outputs: RefCell<Vec<CommandOutput>>,
}

impl FakeRunner {
    fn push_output(&self, output: CommandOutput) {
        self.outputs.borrow_mut().push(output);
    }

    fn command_arguments(&self) -> Vec<Vec<String>> {
        self.commands
            .borrow()
            .iter()
            .map(|command| command.arguments().to_vec())
            .collect()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: &CommandSpec) -> io::Result<CommandOutput> {
        self.commands.borrow_mut().push(command.clone());
        Ok(self
            .outputs
            .borrow_mut()
            .pop()
            .unwrap_or_else(|| CommandOutput::successful("", "")))
    }
}

#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: HashMap<PathBuf, String>,
    fail_writes: bool,
    fail_existing_rename_once: bool,
}

impl AtomicFileSystem for MemoryFileSystem {
    fn write_file(&mut self, path: &Path, contents: &str) -> io::Result<()> {
        if self.fail_writes {
            Err(io::Error::other("simulated write failure"))
        } else {
            self.files.insert(path.to_path_buf(), contents.to_owned());
            Ok(())
        }
    }

    fn rename_file(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if self.fail_existing_rename_once && self.files.contains_key(to) {
            self.fail_existing_rename_once = false;
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination exists",
            ));
        }

        if let Some(contents) = self.files.remove(from) {
            self.files.insert(to.to_path_buf(), contents);
        }
        Ok(())
    }

    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        self.files.remove(path);
        Ok(())
    }
}

#[test]
fn classify_proxy_container_should_require_matching_port_and_dynamic_mount() {
    let config = DinopodConfig::default();
    let paths = ProxyPaths::new(Path::new("/config/dinopod"));
    let expected = ProxyRuntimeSpec::from_config(&config, &paths);
    let healthy = format!(
        "true\ttraefik:v3.6\t{{\"80/tcp\":[{{\"HostPort\":\"80\"}}]}}\t[{{\"Source\":\"{}\",\"Destination\":\"/etc/traefik/dynamic\"}}]\t[\"host.docker.internal:host-gateway\"]",
        paths.resolved_dynamic_config_dir().display()
    );
    let wrong_port = format!(
        "true\ttraefik:v3.6\t{{\"8080/tcp\":[{{\"HostPort\":\"8080\"}}]}}\t[{{\"Source\":\"{}\",\"Destination\":\"/etc/traefik/dynamic\"}}]\t[\"host.docker.internal:host-gateway\"]",
        paths.resolved_dynamic_config_dir().display()
    );

    assert_eq!(
        classify_proxy_container(&healthy, &expected),
        ProxyStatus::Healthy
    );
    assert_eq!(
        classify_proxy_container(&wrong_port, &expected),
        ProxyStatus::NeedsRepair
    );
}

#[test]
fn generated_proxy_compose_should_not_mount_docker_socket() {
    let config = DinopodConfig::default();
    let paths = ProxyPaths::new(Path::new("/config/dinopod"));

    let compose = render_proxy_compose(&config, &paths);

    assert!(!compose.contains("docker.sock"));
    assert!(compose.contains("image: traefik:v3.6"));
    assert!(compose.contains("\"80:80\""));
    assert!(compose.contains("host.docker.internal:host-gateway"));
}

#[test]
fn generated_proxy_compose_should_use_digest_pinned_image_when_configured() {
    let mut config = DinopodConfig::default();
    config.proxy.image = "traefik:v3.6@sha256:abc123".to_owned();
    let paths = ProxyPaths::new(Path::new("/config/dinopod"));

    let compose = render_proxy_compose(&config, &paths);

    assert!(compose.contains("image: traefik:v3.6@sha256:abc123"));
}

#[test]
fn dynamic_route_should_map_hostname_to_proxy_alias_and_internal_port() {
    let config = DinopodConfig::default();
    let names = derive_names("MyApp", "JIRA-123", Path::new("/repo/myapp"), &config)
        .expect("names should derive");

    let route = render_route(&config, &names);

    assert!(route.contains("rule = \"Host(`jira-123-myapp.localhost`)\""));
    assert!(route.contains("url = \"http://myapp-jira-123-app:3000\""));
}

#[test]
fn atomic_route_write_failure_should_leave_previous_route_intact() {
    let path = PathBuf::from("/config/dinopod/proxy/dynamic/jira-123.toml");
    let mut fs = MemoryFileSystem {
        files: HashMap::from([(path.clone(), "old route".to_owned())]),
        fail_writes: true,
        ..MemoryFileSystem::default()
    };
    let mut writer = AtomicWriter::new(&mut fs);

    let error = writer
        .write_atomic(&path, "new route")
        .expect_err("simulated temp write failure should fail");

    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(fs.files.get(&path), Some(&"old route".to_owned()));
}

#[test]
fn atomic_route_write_should_replace_existing_file_when_rename_reports_destination_exists() {
    let path = PathBuf::from("/config/dinopod/proxy/dynamic/jira-123.toml");
    let mut fs = MemoryFileSystem {
        files: HashMap::from([(path.clone(), "old route".to_owned())]),
        fail_writes: false,
        fail_existing_rename_once: true,
    };
    let mut writer = AtomicWriter::new(&mut fs);

    writer
        .write_atomic(&path, "new route")
        .expect("existing route should be replaced");

    assert_eq!(fs.files.get(&path), Some(&"new route".to_owned()));
}

#[test]
fn file_lock_should_prevent_concurrent_proxy_mutations() {
    let lock_path = std::env::temp_dir().join(format!(
        "dinopod-lock-test-{}-{}.lock",
        std::process::id(),
        "proxy"
    ));
    let first = MutationGuard::try_acquire(&lock_path)
        .expect("lock acquisition should not error")
        .expect("first lock should be acquired");
    let second = MutationGuard::try_acquire(&lock_path).expect("second lock should not error");

    assert!(second.is_none());
    drop(first);
    assert!(MutationGuard::try_acquire(&lock_path)
        .expect("third lock should not error")
        .is_some());
}

#[test]
fn stale_file_lock_should_be_recovered_after_stale_age() {
    let lock_path = std::env::temp_dir().join(format!(
        "dinopod-lock-test-{}-{}.lock",
        std::process::id(),
        "stale"
    ));
    std::fs::write(&lock_path, "pid=999999\ncreated_at_unix_seconds=0\n")
        .expect("stale lock fixture should be writable");

    let lock = MutationGuard::try_acquire_with_stale_after(
        &lock_path,
        UNIX_EPOCH + Duration::from_mins(2),
        Duration::from_mins(1),
    )
    .expect("stale lock recovery should not error");

    assert!(lock.is_some());
}

#[test]
fn dropped_guard_should_not_remove_another_process_recovered_lock() {
    let lock_path = std::env::temp_dir().join(format!(
        "dinopod-lock-test-{}-{}.lock",
        std::process::id(),
        "token"
    ));
    let _ = std::fs::remove_file(&lock_path);

    let original = MutationGuard::try_acquire(&lock_path)
        .expect("lock acquisition should not error")
        .expect("original lock should be acquired");

    std::fs::write(
        &lock_path,
        "pid=999999\ncreated_at_unix_seconds=0\ntoken=other-process-token\n",
    )
    .expect("simulated recovered lock should be writable");

    drop(original);

    assert!(
        lock_path.is_file(),
        "drop should not remove a lock file owned by another guard token"
    );
    let _ = std::fs::remove_file(&lock_path);
}

#[test]
fn proxy_start_should_create_network_only_when_network_is_absent() {
    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful("", ""));
    runner.push_output(CommandOutput::successful("", ""));
    runner.push_output(CommandOutput::failed(Some(1), "", "network missing"));
    let manager = ProxyManager::new(&runner);
    let config = DinopodConfig::default();
    let paths = ProxyPaths::new(Path::new("/config/dinopod"));

    let action = manager
        .ensure_proxy(&config, &paths, ProxyStatus::Stopped)
        .expect("proxy startup should run commands");

    assert_eq!(action, ProxyAction::Started);
    assert_eq!(
        runner.command_arguments(),
        [
            ["network", "inspect", "dinopod-proxy"]
                .map(String::from)
                .to_vec(),
            ["network", "create", "dinopod-proxy"]
                .map(String::from)
                .to_vec(),
            [
                "compose",
                "-p",
                "dinopod-proxy",
                "-f",
                "/config/dinopod/proxy/compose.yaml",
                "up",
                "-d",
            ]
            .map(String::from)
            .to_vec(),
        ]
    );
}

#[test]
fn proxy_start_should_reuse_existing_network_before_compose_up() {
    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful("", ""));
    runner.push_output(CommandOutput::successful("", ""));
    let manager = ProxyManager::new(&runner);
    let config = DinopodConfig::default();
    let paths = ProxyPaths::new(Path::new("/config/dinopod"));

    let action = manager
        .ensure_proxy(&config, &paths, ProxyStatus::Stopped)
        .expect("proxy startup should run compose after network inspection");

    assert_eq!(action, ProxyAction::Started);
    assert_eq!(
        runner.command_arguments(),
        [
            ["network", "inspect", "dinopod-proxy"]
                .map(String::from)
                .to_vec(),
            [
                "compose",
                "-p",
                "dinopod-proxy",
                "-f",
                "/config/dinopod/proxy/compose.yaml",
                "up",
                "-d",
            ]
            .map(String::from)
            .to_vec(),
        ]
    );
}

#[test]
fn healthy_proxy_should_be_reused_without_docker_commands() {
    let runner = FakeRunner::default();
    let manager = ProxyManager::new(&runner);
    let config = DinopodConfig::default();
    let paths = ProxyPaths::new(Path::new("/config/dinopod"));

    let action = manager
        .ensure_proxy(&config, &paths, ProxyStatus::Healthy)
        .expect("healthy proxy should be reusable");

    assert_eq!(action, ProxyAction::Reused);
    assert!(runner.command_arguments().is_empty());
}

#[test]
fn proxy_needing_repair_should_remove_container_before_restart() {
    let runner = FakeRunner::default();
    let manager = ProxyManager::new(&runner);
    let config = DinopodConfig::default();
    let paths = ProxyPaths::new(Path::new("/config/dinopod"));

    let action = manager
        .ensure_proxy(&config, &paths, ProxyStatus::NeedsRepair)
        .expect("repair should run commands");

    assert_eq!(action, ProxyAction::Repaired);
    assert_eq!(
        runner.command_arguments()[0],
        ["rm", "-f", "dinopod-traefik"].map(String::from).to_vec()
    );
}

#[test]
fn generated_proxy_compose_should_use_absolute_dynamic_mount_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-proxy-absolute-mount-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let relative_root = Path::new("dinopod-proxy-absolute-mount");
    let previous_dir = std::env::current_dir().expect("current dir should be readable");
    std::env::set_current_dir(&temp_dir).expect("temp dir should become cwd");

    let config = DinopodConfig::default();
    let paths = ProxyPaths::new(relative_root);
    let compose = render_proxy_compose(&config, &paths);
    let resolved = paths.resolved_dynamic_config_dir();

    std::env::set_current_dir(previous_dir).expect("previous cwd should be restored");
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert!(
        compose.contains(&format!("{}:/etc/traefik/dynamic:ro", resolved.display())),
        "compose should bind an absolute dynamic config path, got:\n{compose}"
    );
    assert!(
        resolved.is_absolute(),
        "resolved dynamic config dir should be absolute"
    );
}
