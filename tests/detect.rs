use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use dinopod::config::{DinopodConfig, RuntimeMode};
use dinopod::detect::{
    build_project_profile, compose_has_app_service, detect_package_manager, is_nextjs_project,
    parse_package_json, resolve_dev_script, resolve_runtime_mode, DetectFs, PackageManager,
};
use dinopod::errors::DinopodError;

#[derive(Default)]
struct FakeDetectFs {
    files: BTreeMap<PathBuf, String>,
}

impl FakeDetectFs {
    fn insert(&mut self, path: impl Into<PathBuf>, contents: impl Into<String>) {
        self.files.insert(path.into(), contents.into());
    }
}

impl DetectFs for FakeDetectFs {
    fn file_exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing file"))
    }
}

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn nextjs_package_and_infra_compose_should_select_native_mode() {
    let repo_root = fixture_root("next-package");
    let config = DinopodConfig::default();
    let compose_services = vec!["db".to_owned(), "redis".to_owned()];

    let profile = build_project_profile(
        &config,
        &repo_root,
        &compose_services,
        None,
        &StdDetectFsForFixtures,
    )
    .expect("native profile should resolve");

    assert_eq!(profile.runtime, RuntimeMode::Native);
    assert_eq!(profile.dev_script.as_deref(), Some("dev:all"));
    assert!(profile.is_nextjs);
    assert_eq!(profile.default_app_port, 3000);
}

#[test]
fn explicit_container_runtime_should_override_package_json() {
    let repo_root = fixture_root("next-package");
    let config = DinopodConfig::from_toml_str(r#"runtime = "container""#)
        .expect("runtime config should load");
    let compose_services = vec!["db".to_owned()];

    let profile = build_project_profile(
        &config,
        &repo_root,
        &compose_services,
        None,
        &StdDetectFsForFixtures,
    )
    .expect("container profile should resolve");

    assert_eq!(profile.runtime, RuntimeMode::Container);
    assert!(profile.dev_script.is_none());
    assert_eq!(profile.default_app_port, config.app.internal_port);
}

#[test]
fn missing_dev_scripts_should_list_available_scripts() {
    let package_json = parse_package_json(
        r#"{
            "scripts": { "build": "next build", "lint": "eslint ." }
        }"#,
    )
    .expect("package json should parse");

    let error = resolve_dev_script(&package_json, None, None).expect_err("script is required");

    assert!(matches!(error, DinopodError::DevScriptMissing { .. }));
    assert!(
        error.to_string().contains("build"),
        "expected available scripts in error, got {error}"
    );
}

#[test]
fn unknown_project_type_should_suggest_init_or_container_setup() {
    let config = DinopodConfig::default();
    let error = resolve_runtime_mode(&config, false, false).expect_err("project type required");

    assert!(matches!(error, DinopodError::ProjectTypeUnknown));
    assert!(
        error.to_string().contains("dinopod init"),
        "expected init guidance, got {error}"
    );
}

#[test]
fn invalid_package_json_should_return_parse_error() {
    let repo_root = PathBuf::from("/tmp/dinopod-detect-invalid");
    let mut fs = FakeDetectFs::default();
    fs.insert(repo_root.join("package.json"), "{ this is not valid json");

    let error = build_project_profile(&DinopodConfig::default(), &repo_root, &[], None, &fs)
        .expect_err("invalid json should fail");

    assert!(matches!(error, DinopodError::PackageJsonInvalid(_)));
}

#[test]
fn ambiguous_project_signals_should_require_runtime_config() {
    let config = DinopodConfig::default();
    let error = resolve_runtime_mode(&config, true, true).expect_err("runtime must be explicit");

    assert!(matches!(error, DinopodError::RuntimeModeAmbiguous));
}

#[test]
fn package_manager_should_prefer_pnpm_lockfile() {
    let repo_root = fixture_root("next-package");
    let mut fs = FakeDetectFs::default();
    fs.insert(
        repo_root.join("package.json"),
        std::fs::read_to_string(repo_root.join("package.json")).expect("fixture package.json"),
    );
    fs.insert(repo_root.join("pnpm-lock.yaml"), "lockfileVersion: 9.0\n");

    let profile = build_project_profile(
        &DinopodConfig::default(),
        &repo_root,
        &["db".to_owned()],
        None,
        &fs,
    )
    .expect("profile should resolve");

    assert_eq!(profile.package_manager, Some(PackageManager::Pnpm));
}

#[test]
fn package_manager_should_detect_npm_lockfile() {
    let repo_root = PathBuf::from("/tmp/dinopod-detect-npm");
    let mut fs = FakeDetectFs::default();
    fs.insert(
        repo_root.join("package.json"),
        r#"{"scripts":{"dev":"node dev.js"}}"#,
    );
    fs.insert(repo_root.join("package-lock.json"), "{}");

    assert_eq!(
        detect_package_manager(&repo_root, &fs),
        Some(PackageManager::Npm)
    );
}

#[test]
fn package_manager_should_read_package_json_field_when_lockfile_missing() {
    let repo_root = PathBuf::from("/tmp/dinopod-detect-package-manager-field");
    let mut fs = FakeDetectFs::default();
    fs.insert(
        repo_root.join("package.json"),
        r#"{"packageManager":"pnpm@9.0.0","scripts":{"dev":"next dev"}}"#,
    );

    assert_eq!(
        detect_package_manager(&repo_root, &fs),
        Some(PackageManager::Pnpm)
    );
}

#[test]
fn compose_has_app_service_should_match_configured_name() {
    let services = vec!["web".to_owned(), "db".to_owned()];
    assert!(compose_has_app_service(&services, "web"));
    assert!(!compose_has_app_service(&services, "app"));
}

#[test]
fn is_nextjs_project_should_check_dependencies_and_dev_dependencies() {
    let deps = parse_package_json(r#"{"dependencies":{"next":"15.0.0"}}"#).expect("parse");
    let dev_deps = parse_package_json(r#"{"devDependencies":{"next":"15.0.0"}}"#).expect("parse");
    let other = parse_package_json(r#"{"dependencies":{"react":"19.0.0"}}"#).expect("parse");

    assert!(is_nextjs_project(&deps));
    assert!(is_nextjs_project(&dev_deps));
    assert!(!is_nextjs_project(&other));
}

struct StdDetectFsForFixtures;

impl DetectFs for StdDetectFsForFixtures {
    fn file_exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}
