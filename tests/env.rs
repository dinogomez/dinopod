use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use dinopod::config::SettingsConfig;
use dinopod::env::{
    allocate_port_plan, copy_env_files_on_create, is_dotenv_file_name, load_merged_env,
    refresh_env_files, render_env_overlay, should_install_dependencies, sync_missing_env_files,
    write_env_overlay, EnvFs, OverlayContext, PortBinder, PortPlan, ENV_FILE_NAMES,
};
use dinopod::errors::DinopodError;

#[derive(Default)]
struct FakeEnvFs {
    files: RefCell<BTreeMap<PathBuf, String>>,
    dirs: BTreeSet<PathBuf>,
    symlinks: BTreeSet<PathBuf>,
}

impl FakeEnvFs {
    fn insert_file(&self, path: impl Into<PathBuf>, contents: impl Into<String>) {
        self.files.borrow_mut().insert(path.into(), contents.into());
    }

    fn insert_dir(&mut self, path: impl Into<PathBuf>) {
        self.dirs.insert(path.into());
    }

    fn insert_symlink(&mut self, path: impl Into<PathBuf>) {
        self.symlinks.insert(path.into());
    }

    fn read(&self, path: &Path) -> Option<String> {
        self.files.borrow().get(path).cloned()
    }
}

impl EnvFs for FakeEnvFs {
    fn path_exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path) || self.symlinks.contains(path)
    }

    fn dir_exists(&self, path: &Path) -> bool {
        self.dirs.contains(path)
    }

    fn is_symlink(&self, path: &Path) -> bool {
        self.symlinks.contains(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing file"))
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let _ = path;
        Ok(())
    }

    fn write_file(&self, path: &Path, contents: &str, _mode: u32) -> io::Result<()> {
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), contents.to_owned());
        Ok(())
    }

    fn copy_regular_file(&self, from: &Path, to: &Path, mode: u32) -> io::Result<()> {
        let contents = self.read_to_string(from)?;
        self.write_file(to, &contents, mode)
    }

    fn list_dotenv_files(&self, root: &Path) -> io::Result<Vec<PathBuf>> {
        let mut files = self
            .files
            .borrow()
            .keys()
            .filter(|path| {
                path.parent() == Some(root)
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(is_dotenv_file_name)
            })
            .cloned()
            .collect::<Vec<_>>();
        files.extend(
            self.symlinks
                .iter()
                .filter(|path| {
                    path.parent() == Some(root)
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(is_dotenv_file_name)
                })
                .cloned(),
        );
        files.sort();
        files.dedup();
        Ok(files)
    }
}

struct FakePortBinder {
    blocked: HashSet<u16>,
}

impl FakePortBinder {
    fn block(blocked: impl IntoIterator<Item = u16>) -> Self {
        Self {
            blocked: blocked.into_iter().collect(),
        }
    }
}

impl PortBinder for FakePortBinder {
    fn can_bind(&self, port: u16) -> bool {
        !self.blocked.contains(&port)
    }
}

#[test]
fn sync_missing_env_files_should_copy_all_dotenv_files_from_source() {
    let primary = PathBuf::from("/primary");
    let worktree = PathBuf::from("/worktree");
    let fs = FakeEnvFs::default();
    fs.insert_file(primary.join(".env.local"), "BETTER_AUTH_SECRET=secret\n");
    fs.insert_file(primary.join(".env-full"), "EXTRA=value\n");
    fs.insert_file(primary.join("secrets.env"), "API_KEY=abc\n");
    fs.insert_file(worktree.join(".env.example"), "TEMPLATE=1\n");

    sync_missing_env_files(&primary, &worktree, &SettingsConfig::default(), &fs)
        .expect("sync should succeed");

    assert_eq!(
        fs.read(&worktree.join(".env.local")).as_deref(),
        Some("BETTER_AUTH_SECRET=secret\n")
    );
    assert_eq!(
        fs.read(&worktree.join(".env-full")).as_deref(),
        Some("EXTRA=value\n")
    );
    assert_eq!(
        fs.read(&worktree.join("secrets.env")).as_deref(),
        Some("API_KEY=abc\n")
    );
    assert_eq!(
        fs.read(&worktree.join(".env.example")).as_deref(),
        Some("TEMPLATE=1\n")
    );
}

#[test]
fn copy_env_files_on_create_should_copy_known_files() {
    let primary = PathBuf::from("/primary");
    let worktree = PathBuf::from("/worktree");
    let fs = FakeEnvFs::default();
    fs.insert_file(
        primary.join(".env.local"),
        "DATABASE_URL=postgres://localhost:5432/app\n",
    );
    fs.insert_file(primary.join(".env"), "NODE_ENV=development\n");

    copy_env_files_on_create(&primary, &worktree, &SettingsConfig::default(), &fs)
        .expect("copy should succeed");

    assert_eq!(
        fs.read(&worktree.join(".env.local")).as_deref(),
        Some("DATABASE_URL=postgres://localhost:5432/app\n")
    );
    assert_eq!(
        fs.read(&worktree.join(".env")).as_deref(),
        Some("NODE_ENV=development\n")
    );
}

#[test]
fn overlay_should_set_database_url_and_ticket_url() {
    let mut existing = BTreeMap::new();
    existing.insert(
        "DATABASE_URL".to_owned(),
        "postgresql://postgres:secret@localhost:5432/promptwise".to_owned(),
    );

    let overlay = render_env_overlay(
        &OverlayContext {
            ticket_url: "http://jira-123-promptwise.localhost".to_owned(),
            port_plan: PortPlan {
                app_host_port: 31_234,
                postgres_host_port: Some(54_321),
                redis_host_port: Some(63_210),
            },
        },
        &existing,
    );

    assert!(
        overlay.contains("DATABASE_URL=postgresql://postgres:secret@localhost:54321/promptwise")
    );
    assert!(overlay.contains("NEXT_PUBLIC_APP_URL=http://jira-123-promptwise.localhost"));
    assert!(overlay.contains("PORT=31234"));
}

#[test]
fn port_plan_should_be_stable_for_same_ticket_and_vary_by_ticket() {
    let services = vec!["db".to_owned(), "redis".to_owned()];
    let binder = FakePortBinder::block([]);

    let first =
        allocate_port_plan("promptwise", "jira-123", &services, &binder).expect("first plan");
    let second =
        allocate_port_plan("promptwise", "jira-123", &services, &binder).expect("second plan");
    let other =
        allocate_port_plan("promptwise", "jira-456", &services, &binder).expect("other plan");

    assert_eq!(first, second);
    assert_ne!(first.app_host_port, other.app_host_port);
}

#[test]
fn port_plan_should_probe_next_free_port_on_collision() {
    let services = vec!["db".to_owned()];
    let seed_plan = allocate_port_plan(
        "promptwise",
        "jira-123",
        &services,
        &FakePortBinder::block([]),
    )
    .expect("seed plan");
    let blocked = FakePortBinder::block([seed_plan.postgres_host_port.expect("postgres port")]);
    let probed =
        allocate_port_plan("promptwise", "jira-123", &services, &blocked).expect("probed plan");

    assert_ne!(
        seed_plan.postgres_host_port, probed.postgres_host_port,
        "collision should advance to the next free port"
    );
}

#[test]
fn should_install_dependencies_should_skip_when_node_modules_exists_or_flag_set() {
    let worktree = PathBuf::from("/worktree");
    let mut fs = FakeEnvFs::default();

    assert!(should_install_dependencies(&worktree, false, &fs));

    fs.insert_dir(worktree.join("node_modules"));
    assert!(!should_install_dependencies(&worktree, false, &fs));

    assert!(!should_install_dependencies(
        &worktree,
        true,
        &FakeEnvFs::default()
    ));
}

#[test]
fn refresh_env_should_add_missing_keys_without_overwriting_worktree_values() {
    let primary = PathBuf::from("/primary");
    let worktree = PathBuf::from("/worktree");
    let fs = FakeEnvFs::default();
    fs.insert_file(
        primary.join(".env.local"),
        "DATABASE_URL=postgres://localhost:5432/app\nNEW_KEY=from-primary\n",
    );
    fs.insert_file(
        worktree.join(".env.local"),
        "DATABASE_URL=postgres://localhost:9999/custom\n",
    );

    refresh_env_files(&primary, &worktree, &fs).expect("refresh should succeed");

    let updated = fs.read(&worktree.join(".env.local")).expect("worktree env");
    assert!(updated.contains("DATABASE_URL=postgres://localhost:9999/custom"));
    assert!(updated.contains("NEW_KEY=from-primary"));
}

#[test]
fn load_merged_env_should_apply_overlay_over_copied_files() {
    let worktree = PathBuf::from("/worktree");
    let fs = FakeEnvFs::default();
    fs.insert_file(worktree.join(".env.local"), "PORT=3000\n");
    fs.insert_file(
        worktree.join(".dinopod/env.overlay"),
        "PORT=31234\nNEXT_PUBLIC_APP_URL=http://ticket.localhost\n",
    );

    let merged = load_merged_env(&worktree, &worktree.join(".dinopod/env.overlay"), &fs)
        .expect("merged env");

    assert_eq!(merged.get("PORT"), Some(&"31234".to_owned()));
    assert_eq!(
        merged.get("NEXT_PUBLIC_APP_URL"),
        Some(&"http://ticket.localhost".to_owned())
    );
}

#[test]
fn write_env_overlay_should_persist_rendered_overlay() {
    let worktree = PathBuf::from("/worktree");
    let fs = FakeEnvFs::default();
    let path = write_env_overlay(
        &worktree,
        &OverlayContext {
            ticket_url: "http://ticket.localhost".to_owned(),
            port_plan: PortPlan {
                app_host_port: 31_111,
                postgres_host_port: None,
                redis_host_port: Some(63_111),
            },
        },
        &BTreeMap::new(),
        &fs,
    )
    .expect("overlay write");

    assert_eq!(path, worktree.join(".dinopod/env.overlay"));
    let contents = fs.read(&path).expect("overlay contents");
    assert!(contents.contains("REDIS_URL=redis://localhost:63111"));
}

#[test]
fn copy_env_files_on_create_should_reject_symlinks() {
    let primary = PathBuf::from("/primary");
    let worktree = PathBuf::from("/worktree");
    let mut fs = FakeEnvFs::default();
    fs.insert_symlink(primary.join(".env.local"));

    let error = copy_env_files_on_create(&primary, &worktree, &SettingsConfig::default(), &fs)
        .expect_err("symlink should be rejected");

    assert!(matches!(error, DinopodError::EnvSymlinkRejected { .. }));
}

#[test]
fn port_range_exhaustion_should_return_clear_error() {
    let services = vec!["db".to_owned()];
    let blocked: Vec<u16> = (54_000..=54_999).collect();
    let error = allocate_port_plan(
        "promptwise",
        "jira-123",
        &services,
        &FakePortBinder::block(blocked),
    )
    .expect_err("range should exhaust");

    assert!(matches!(error, DinopodError::PortRangeExhausted { .. }));
}

#[test]
fn is_dotenv_file_name_should_match_any_filename_containing_env() {
    assert!(is_dotenv_file_name(".env"));
    assert!(is_dotenv_file_name(".env.local"));
    assert!(is_dotenv_file_name(".env-full"));
    assert!(is_dotenv_file_name("secrets.env"));
    assert!(!is_dotenv_file_name("environment.ts"));
    assert!(!is_dotenv_file_name("package.json"));
}

#[test]
fn env_file_names_should_cover_standard_nextjs_env_files() {
    assert!(ENV_FILE_NAMES.contains(&".env.local"));
    assert!(ENV_FILE_NAMES.contains(&".env.development.local"));
}
