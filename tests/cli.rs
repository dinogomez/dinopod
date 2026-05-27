use std::process::Command;
use std::{fs, process};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn help_should_list_core_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("--help")
        .output()
        .expect("dinopod binary should run");

    let stdout = String::from_utf8(output.stdout).expect("help output should be valid UTF-8");

    assert!(
        output.status.success()
            && stdout.contains("dev")
            && stdout.contains("init")
            && stdout.contains("list"),
        "help should succeed and list the core commands"
    );
}

#[test]
fn no_command_should_display_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .output()
        .expect("dinopod binary should run");

    let stdout = String::from_utf8(output.stdout).expect("help output should be valid UTF-8");

    assert!(
        output.status.success() && stdout.contains("Usage: dinopod") && stdout.contains("dev"),
        "no command should render help and exit successfully"
    );
}

#[test]
fn init_should_write_default_config_in_current_directory() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-init-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("init")
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod init should run");

    let config = fs::read_to_string(temp_dir.join("dinopod.toml"))
        .expect("dinopod init should write config");

    assert!(output.status.success());
    assert!(config.contains("[app]"));
    assert!(config.contains("image = \"traefik:v3.6\""));
}

#[test]
fn init_should_not_overwrite_existing_config() {
    let temp_dir =
        std::env::temp_dir().join(format!("dinopod-cli-init-existing-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    fs::write(temp_dir.join("dinopod.toml"), "existing config\n")
        .expect("existing config should be written");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("init")
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod init should run");

    let config = fs::read_to_string(temp_dir.join("dinopod.toml"))
        .expect("dinopod config should remain readable");

    assert!(!output.status.success());
    assert_eq!(config, "existing config\n");
}

#[test]
fn dev_should_report_missing_git_before_trying_lifecycle_commands() {
    let temp_dir =
        std::env::temp_dir().join(format!("dinopod-cli-preflight-test-{}", process::id()));
    let bin_dir = temp_dir.join("bin");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&bin_dir).expect("temp bin dir should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("dev")
        .arg("JIRA-123")
        .env("PATH", &bin_dir)
        .env("DINOPOD_CONFIG_DIR", temp_dir.join("config"))
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod dev should run");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(!output.status.success());
    assert!(stderr.contains("missing required dependency: git"));
}

#[cfg(unix)]
#[test]
fn dev_from_subdirectory_should_use_primary_repo_root_and_root_config() {
    let fixture = FakeCliRepo::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("dev")
        .arg("JIRA-123")
        .env("PATH", fixture.path_env())
        .env("DINOPOD_CONFIG_DIR", &fixture.config_root)
        .env("DINOPOD_FAKE_LOG", &fixture.fake_log)
        .env("DINOPOD_REPO_ROOT", &fixture.repo)
        .current_dir(&fixture.subdir)
        .output()
        .expect("dinopod dev should run");

    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(output.status.success());
    assert!(stdout.contains("project: myrepo-jira-123"));
    assert!(stdout.contains("url: http://jira-123-myrepo.localhost:18081"));
    assert!(stderr.contains("fixed host port"));
    let log = fs::read_to_string(&fixture.fake_log).expect("fake command log should be readable");
    assert!(log.contains(&format!(
        "git worktree add -b jira-123 {}",
        fixture.worktree_root.join("myrepo-jira-123").display()
    )));
}

#[test]
fn list_with_empty_state_should_succeed_without_docker() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-list-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("list")
        .env("DINOPOD_CONFIG_DIR", &temp_dir)
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod list should run");

    assert!(output.status.success());
}

#[test]
fn list_should_fail_when_lifecycle_lock_is_held() {
    let temp_dir = std::env::temp_dir().join(format!("dinopod-cli-lock-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    fs::write(temp_dir.join("dinopod.lock"), "held-by-test\n")
        .expect("lock file should be written");

    let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
        .arg("list")
        .env("DINOPOD_CONFIG_DIR", &temp_dir)
        .current_dir(&temp_dir)
        .output()
        .expect("dinopod list should run");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");

    assert!(!output.status.success());
    assert!(stderr.contains("another dinopod command is already running"));
}

#[cfg(unix)]
#[derive(Debug)]
struct FakeCliRepo {
    repo: std::path::PathBuf,
    subdir: std::path::PathBuf,
    fake_bin: std::path::PathBuf,
    fake_log: std::path::PathBuf,
    config_root: std::path::PathBuf,
    worktree_root: std::path::PathBuf,
}

#[cfg(unix)]
impl FakeCliRepo {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("dinopod-cli-subdir-repo-test-{}", process::id()));
        let repo = root.join("myrepo");
        let fixture = Self {
            subdir: repo.join("src"),
            fake_bin: root.join("bin"),
            fake_log: root.join("commands.log"),
            config_root: root.join("config"),
            worktree_root: root.join("worktrees"),
            repo,
        };
        fixture.write_files(&root);
        fixture
    }

    fn path_env(&self) -> String {
        format!(
            "{}:{}",
            self.fake_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn write_files(&self, root: &std::path::Path) {
        let _ = fs::remove_dir_all(root);
        fs::create_dir_all(&self.subdir).expect("repo subdir should be created");
        fs::create_dir_all(&self.fake_bin).expect("fake bin should be created");
        fs::write(
            self.repo.join("docker-compose.yml"),
            "services:\n  app:\n    image: example\n",
        )
        .expect("compose fixture should be written");
        fs::write(self.repo.join("dinopod.toml"), self.config())
            .expect("config fixture should be written");
        write_executable(&self.fake_bin.join("git"), FAKE_GIT);
        write_executable(&self.fake_bin.join("docker"), FAKE_DOCKER);
    }

    fn config(&self) -> String {
        format!(
            concat!(
                "[app]\n",
                "service = \"app\"\n",
                "internal_port = 3000\n",
                "compose_file = \"docker-compose.yml\"\n",
                "default_branch = \"main\"\n\n",
                "[worktree]\n",
                "root = \"{}\"\n\n",
                "[proxy]\n",
                "host_suffix = \"localhost\"\n",
                "network = \"dinopod-test-proxy\"\n",
                "container_name = \"dinopod-test-traefik\"\n",
                "http_port = 18081\n",
                "image = \"traefik:v3.6\"\n",
            ),
            self.worktree_root.display()
        )
    }
}

#[cfg(unix)]
const FAKE_GIT: &str = r#"#!/bin/sh
printf 'git %s\n' "$*" >> "$DINOPOD_FAKE_LOG"
if [ "$1" = "--version" ]; then
  echo "git version fake"
  exit 0
fi
if [ "$1" = "rev-parse" ] && [ "$2" = "--is-inside-work-tree" ]; then
  echo "true"
  exit 0
fi
if [ "$1" = "worktree" ] && [ "$2" = "list" ]; then
  echo "worktree $DINOPOD_REPO_ROOT"
  echo "branch refs/heads/main"
  exit 0
fi
if [ "$1" = "rev-parse" ] && [ "$2" = "--verify" ]; then
  exit 1
fi
if [ "$1" = "worktree" ] && [ "$2" = "add" ]; then
  mkdir -p "$5"
  cp "$DINOPOD_REPO_ROOT/docker-compose.yml" "$5/docker-compose.yml"
  exit 0
fi
exit 0
"#;

#[cfg(unix)]
const FAKE_DOCKER: &str = r#"#!/bin/sh
printf 'docker %s\n' "$*" >> "$DINOPOD_FAKE_LOG"
if [ "$1" = "--version" ] || [ "$1" = "info" ]; then
  exit 0
fi
if [ "$1" = "compose" ] && [ "$2" = "version" ]; then
  exit 0
fi
if [ "$1" = "ps" ]; then
  exit 0
fi
if [ "$1" = "inspect" ]; then
  exit 1
fi
if [ "$1" = "network" ] && [ "$2" = "inspect" ]; then
  exit 1
fi
if [ "$1" = "network" ] && [ "$2" = "create" ]; then
  exit 0
fi
if [ "$1" = "compose" ] && [ "$4" = "config" ]; then
  echo '{"services":{"app":{"ports":[{"published":"3000"}]}}}'
  exit 0
fi
exit 0
"#;

#[cfg(unix)]
fn write_executable(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).expect("fake executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("fake executable metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("fake executable should be executable");
}
