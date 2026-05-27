use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn crate_should_forbid_unsafe_code() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml should be readable");
    let lib = fs::read_to_string("src/lib.rs").expect("src/lib.rs should be readable");
    let main = fs::read_to_string("src/main.rs").expect("src/main.rs should be readable");

    assert!(
        manifest.contains("unsafe_code = \"forbid\"")
            && lib.contains("#![forbid(unsafe_code)]")
            && main.contains("#![forbid(unsafe_code)]"),
        "crate should forbid unsafe code at manifest and crate roots"
    );
}

#[test]
fn dependency_policy_should_reject_unknown_sources() {
    let deny = fs::read_to_string("deny.toml").expect("deny.toml should be readable");

    assert!(
        deny.contains("unknown-registry = \"deny\"") && deny.contains("unknown-git = \"deny\""),
        "dependency policy should deny unknown registries and git sources"
    );
}

#[test]
fn source_should_not_use_broad_lint_allowances() {
    for path in rust_source_paths(Path::new("src")) {
        let content = fs::read_to_string(&path).expect("source file should be readable");
        assert!(
            !content.contains("#[allow(") && !content.contains("#![allow("),
            "{} should use justified #[expect(...)] instead of broad #[allow(...)]",
            path.display()
        );
    }
}

fn rust_source_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_rust_source_paths(root, &mut paths);
    paths
}

fn collect_rust_source_paths(directory: &Path, paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("source directory should be readable") {
        let entry = entry.expect("source directory entry should be readable");
        let path = entry.path();

        if path.is_dir() {
            collect_rust_source_paths(&path, paths);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
}
