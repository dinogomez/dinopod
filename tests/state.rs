use std::cell::RefCell;
use std::path::{Path, PathBuf};

use dinopod::compose::ComposeInspection;
use dinopod::config::DinopodConfig;
use dinopod::errors::DinopodError;
use dinopod::lifecycle::{DevSummary, LifecycleManager, LifecyclePorts};
use dinopod::state::{EnvironmentRecord, EnvironmentStatus, InMemoryStateStore, StateStore};

#[derive(Debug, Default)]
struct FakePorts {
    calls: RefCell<Vec<String>>,
    compose_files: RefCell<Vec<Vec<PathBuf>>>,
    running_projects: RefCell<Vec<String>>,
    fail_compose_up: bool,
}

impl FakePorts {
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    fn compose_files(&self) -> Vec<Vec<PathBuf>> {
        self.compose_files.borrow().clone()
    }
}

impl LifecyclePorts for FakePorts {
    fn ensure_worktree(
        &self,
        _repo_root: &Path,
        worktree_path: &Path,
        branch: &str,
        default_branch: &str,
    ) -> Result<(), DinopodError> {
        self.calls.borrow_mut().push(format!(
            "worktree:{}:{}:{}",
            worktree_path.display(),
            branch,
            default_branch
        ));
        Ok(())
    }

    fn write_compose_override(&self, path: &Path, _contents: &str) -> Result<(), DinopodError> {
        self.calls
            .borrow_mut()
            .push(format!("write-compose:{}", path.display()));
        Ok(())
    }

    fn write_route(&self, path: &Path, _contents: &str) -> Result<(), DinopodError> {
        self.calls
            .borrow_mut()
            .push(format!("write-route:{}", path.display()));
        Ok(())
    }

    fn remove_route(&self, path: &Path) -> Result<(), DinopodError> {
        self.calls
            .borrow_mut()
            .push(format!("remove-route:{}", path.display()));
        Ok(())
    }

    fn ensure_proxy(&self) -> Result<(), DinopodError> {
        self.calls.borrow_mut().push("ensure-proxy".to_owned());
        Ok(())
    }

    fn compose_up(
        &self,
        project: &str,
        compose_files: &[PathBuf],
    ) -> Result<ComposeInspection, DinopodError> {
        self.calls
            .borrow_mut()
            .push(format!("compose-up:{project}"));
        self.compose_files.borrow_mut().push(compose_files.to_vec());
        if self.fail_compose_up {
            return Err(DinopodError::DockerCommandFailed {
                args: vec!["compose".to_owned(), "up".to_owned()],
                exit_code: Some(1),
                stderr: "compose failed".to_owned(),
            });
        }
        self.running_projects.borrow_mut().push(project.to_owned());
        Ok(ComposeInspection::default())
    }

    fn compose_stop(&self, project: &str) -> Result<(), DinopodError> {
        self.calls
            .borrow_mut()
            .push(format!("compose-stop:{project}"));
        self.running_projects
            .borrow_mut()
            .retain(|candidate| candidate != project);
        Ok(())
    }

    fn compose_down(&self, project: &str, volumes: bool) -> Result<(), DinopodError> {
        self.calls
            .borrow_mut()
            .push(format!("compose-down:{project}:{volumes}"));
        self.running_projects
            .borrow_mut()
            .retain(|candidate| candidate != project);
        Ok(())
    }

    fn remove_worktree(&self, path: &Path) -> Result<(), DinopodError> {
        self.calls
            .borrow_mut()
            .push(format!("remove-worktree:{}", path.display()));
        Ok(())
    }

    fn project_is_running(&self, project: &str) -> Result<bool, DinopodError> {
        Ok(self
            .running_projects
            .borrow()
            .iter()
            .any(|candidate| candidate == project))
    }
}

fn manager<'a>(
    ports: &'a FakePorts,
    state: &'a InMemoryStateStore,
) -> LifecycleManager<'a, FakePorts, InMemoryStateStore> {
    manager_with_config(ports, state, DinopodConfig::default())
}

fn manager_with_config<'a>(
    ports: &'a FakePorts,
    state: &'a InMemoryStateStore,
    config: DinopodConfig,
) -> LifecycleManager<'a, FakePorts, InMemoryStateStore> {
    LifecycleManager::new(
        config,
        "MyApp",
        Path::new("/repo/myapp"),
        Path::new("/config/dinopod"),
        ports,
        state,
    )
}

#[test]
fn dev_should_orchestrate_environment_creation_and_write_state() {
    let ports = FakePorts::default();
    let state = InMemoryStateStore::default();
    let manager = manager(&ports, &state);

    let summary = manager.dev("JIRA-123").expect("dev should orchestrate");

    assert_eq!(
        summary,
        DevSummary {
            worktree_path: PathBuf::from("/repo/.dinopod-worktrees/myapp-jira-123"),
            project: "myapp-jira-123".to_owned(),
            url: "http://jira-123.localhost".to_owned(),
            warnings: Vec::new(),
        }
    );
    assert_eq!(
        ports.calls(),
        [
            "worktree:/repo/.dinopod-worktrees/myapp-jira-123:jira-123:main",
            "write-compose:/repo/.dinopod-worktrees/myapp-jira-123/.dinopod/compose.override.yml",
            "ensure-proxy",
            "compose-up:myapp-jira-123",
            "write-route:/config/dinopod/proxy/dynamic/myapp-jira-123.toml",
        ]
    );
    assert_eq!(
        state
            .load()
            .expect("state should load")
            .get("myapp-jira-123")
            .expect("record should exist")
            .status,
        EnvironmentStatus::Running
    );
}

#[test]
fn dev_should_start_compose_with_ticket_worktree_compose_file() {
    let ports = FakePorts::default();
    let state = InMemoryStateStore::default();
    let manager = manager(&ports, &state);

    manager.dev("JIRA-123").expect("dev should orchestrate");

    assert_eq!(
        ports.compose_files()[0][0],
        PathBuf::from("/repo/.dinopod-worktrees/myapp-jira-123/docker-compose.yml")
    );
}

#[test]
fn dev_should_include_configured_proxy_port_in_url_when_not_default_http() {
    let ports = FakePorts::default();
    let state = InMemoryStateStore::default();
    let mut config = DinopodConfig::default();
    config.proxy.http_port = 18080;
    let manager = manager_with_config(&ports, &state, config);

    let summary = manager.dev("JIRA-123").expect("dev should orchestrate");

    assert_eq!(summary.url, "http://jira-123.localhost:18080");
}

#[test]
fn dev_should_not_leave_route_or_state_when_compose_up_fails() {
    let ports = FakePorts {
        fail_compose_up: true,
        ..FakePorts::default()
    };
    let state = InMemoryStateStore::default();
    let manager = manager(&ports, &state);

    let error = manager
        .dev("JIRA-123")
        .expect_err("compose failure should fail dev");

    assert!(matches!(error, DinopodError::DockerCommandFailed { .. }));
    assert!(!ports
        .calls()
        .iter()
        .any(|call| call.starts_with("write-route:")));
    assert!(state.load().expect("state should load").is_empty());
}

#[test]
fn list_should_reconcile_missing_docker_project_as_stale() {
    let ports = FakePorts::default();
    let state = InMemoryStateStore::default();
    state
        .save(vec![EnvironmentRecord {
            project: "myapp-jira-123".to_owned(),
            ticket: "JIRA-123".to_owned(),
            host: "jira-123.localhost".to_owned(),
            url: "http://jira-123.localhost".to_owned(),
            worktree_path: PathBuf::from("/repo/.dinopod-worktrees/myapp-jira-123"),
            route_path: PathBuf::from("/config/dinopod/proxy/dynamic/myapp-jira-123.toml"),
            status: EnvironmentStatus::Running,
        }])
        .expect("state save should work");
    let manager = manager(&ports, &state);

    let records = manager.list().expect("list should reconcile state");

    assert_eq!(records[0].status, EnvironmentStatus::Stale);
}

#[test]
fn down_should_remove_route_and_preserve_volumes_by_default() {
    let ports = FakePorts::default();
    let state = InMemoryStateStore::default();
    let manager = manager(&ports, &state);
    manager.dev("JIRA-123").expect("dev should create state");

    manager
        .down("JIRA-123", false)
        .expect("down should remove containers and route");

    assert!(ports
        .calls()
        .contains(&"compose-down:myapp-jira-123:false".to_owned()));
    assert!(ports
        .calls()
        .contains(&"remove-route:/config/dinopod/proxy/dynamic/myapp-jira-123.toml".to_owned()));
    assert_eq!(
        state
            .load()
            .expect("state should load")
            .get("myapp-jira-123")
            .expect("record should remain")
            .status,
        EnvironmentStatus::Down
    );
}

#[test]
fn rm_should_require_confirmation_before_removing_worktree() {
    let ports = FakePorts::default();
    let state = InMemoryStateStore::default();
    let manager = manager(&ports, &state);
    manager.dev("JIRA-123").expect("dev should create state");

    let error = manager
        .rm("JIRA-123", false)
        .expect_err("rm without confirmation should fail");

    assert!(matches!(error, DinopodError::ConfirmationRequired { .. }));
}

#[test]
fn forced_rm_should_remove_route_project_worktree_and_state() {
    let ports = FakePorts::default();
    let state = InMemoryStateStore::default();
    let manager = manager(&ports, &state);
    manager.dev("JIRA-123").expect("dev should create state");

    manager.rm("JIRA-123", true).expect("forced rm should run");

    assert!(ports
        .calls()
        .contains(&"compose-down:myapp-jira-123:false".to_owned()));
    assert!(ports
        .calls()
        .contains(&"remove-worktree:/repo/.dinopod-worktrees/myapp-jira-123".to_owned()));
    assert!(state.load().expect("state should load").is_empty());
}
