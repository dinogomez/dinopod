use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires Docker and mutates local Docker resources"]
fn docker_smoke_should_start_two_ticket_environments_and_cleanup() -> Result<(), Box<dyn Error>> {
    let smoke = SmokeEnv::new()?;
    smoke.init_repo()?;

    let first = smoke.dinopod(["dev", "JIRA-123"])?;
    let second = smoke.dinopod(["dev", "JIRA-456"])?;
    let rerun = smoke.dinopod(["dev", "JIRA-123"])?;

    let first_stdout = String::from_utf8(first.stdout)?;
    let second_stdout = String::from_utf8(second.stdout)?;
    let rerun_stdout = String::from_utf8(rerun.stdout)?;

    assert!(first_stdout.contains("  🦕 project  dinopod-smoke-repo-jira-123"));
    assert!(second_stdout.contains("  🦕 project  dinopod-smoke-repo-jira-456"));
    assert!(rerun_stdout.contains("http://jira-123-dinopod-smoke-repo.localhost"));

    wait_for_response(
        smoke.proxy_port,
        "jira-123-dinopod-smoke-repo.localhost",
        "dinopod",
    )?;
    wait_for_response(
        smoke.proxy_port,
        "jira-456-dinopod-smoke-repo.localhost",
        "dinopod",
    )?;

    Ok(())
}

#[derive(Debug)]
struct SmokeEnv {
    root: PathBuf,
    repo: PathBuf,
    config_root: PathBuf,
    worktree_root: PathBuf,
    proxy_network: String,
    proxy_container: String,
    proxy_port: u16,
}

impl SmokeEnv {
    fn new() -> Result<Self, Box<dyn Error>> {
        let suffix = std::process::id();
        let root = std::env::temp_dir().join(format!("dinopod-e2e-{suffix}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;

        Ok(Self {
            repo: root.join("dinopod-smoke-repo"),
            config_root: root.join("config"),
            worktree_root: root.join("worktrees"),
            proxy_network: format!("dinopod-e2e-proxy-{suffix}"),
            proxy_container: format!("dinopod-e2e-traefik-{suffix}"),
            proxy_port: 18000 + (suffix % 1000) as u16,
            root,
        })
    }

    fn init_repo(&self) -> Result<(), Box<dyn Error>> {
        fs::create_dir_all(&self.repo)?;
        fs::copy(
            "tests/fixtures/basic-compose/compose.yaml",
            self.repo.join("docker-compose.yml"),
        )?;
        fs::write(self.repo.join("dinopod.toml"), self.config())?;
        run_success(
            Command::new("git")
                .args(["init", "-b", "main"])
                .current_dir(&self.repo),
        )?;
        run_success(
            Command::new("git")
                .args(["config", "user.email", "dinopod@example.test"])
                .current_dir(&self.repo),
        )?;
        run_success(
            Command::new("git")
                .args(["config", "user.name", "Dinopod Smoke"])
                .current_dir(&self.repo),
        )?;
        run_success(
            Command::new("git")
                .args(["add", "."])
                .current_dir(&self.repo),
        )?;
        run_success(
            Command::new("git")
                .args(["commit", "-m", "smoke fixture"])
                .current_dir(&self.repo),
        )?;
        Ok(())
    }

    fn dinopod<I, S>(&self, args: I) -> Result<Output, Box<dyn Error>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = Command::new(env!("CARGO_BIN_EXE_dinopod"))
            .args(args)
            .env("DINOPOD_CONFIG_DIR", &self.config_root)
            .current_dir(&self.repo)
            .output()?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(format!(
                "dinopod failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into())
        }
    }

    fn config(&self) -> String {
        format!(
            concat!(
                "[app]\n",
                "service = \"app\"\n",
                "internal_port = 5678\n",
                "compose_file = \"docker-compose.yml\"\n",
                "default_branch = \"main\"\n\n",
                "[worktree]\n",
                "root = \"{}\"\n\n",
                "[proxy]\n",
                "host_suffix = \"localhost\"\n",
                "network = \"{}\"\n",
                "container_name = \"{}\"\n",
                "http_port = {}\n",
                "image = \"traefik:v3.6\"\n",
            ),
            self.worktree_root.display(),
            self.proxy_network,
            self.proxy_container,
            self.proxy_port
        )
    }
}

impl Drop for SmokeEnv {
    fn drop(&mut self) {
        for ticket in ["JIRA-123", "JIRA-456"] {
            let _ = Command::new(env!("CARGO_BIN_EXE_dinopod"))
                .args(["rm", ticket, "--yes"])
                .env("DINOPOD_CONFIG_DIR", &self.config_root)
                .current_dir(&self.repo)
                .output();
        }
        let proxy_compose = self.config_root.join("proxy").join("compose.yaml");
        let proxy_compose_arg = proxy_compose.display().to_string();
        let _ = Command::new("docker")
            .args([
                "compose",
                "-p",
                self.proxy_network.as_str(),
                "-f",
                proxy_compose_arg.as_str(),
                "down",
            ])
            .output();
        let _ = Command::new("docker")
            .args(["network", "rm", &self.proxy_network])
            .output();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn wait_for_response(port: u16, host: &str, needle: &str) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(response) = http_get(port, host) {
            if response.contains(needle) {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(500));
    }

    Err(format!("timed out waiting for response from {host}:{port}").into())
}

fn http_get(port: u16, host: &str) -> Result<String, Box<dyn Error>> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn run_success(command: &mut Command) -> Result<(), Box<dyn Error>> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
}
