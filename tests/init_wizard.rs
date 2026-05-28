use std::fs;
use std::process::Command;

use dinopod::init_wizard::{detect_compose_file, detect_default_branch};

#[test]
fn detect_compose_file_should_prefer_existing_compose_yml() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-init-detect-compose-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    fs::write(temp_dir.join("compose.yml"), "services: {}\n").expect("compose file should exist");

    assert_eq!(detect_compose_file(&temp_dir), "compose.yml");
}

#[test]
fn detect_compose_file_should_fall_back_to_docker_compose_yml() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-init-detect-compose-default-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");

    assert_eq!(detect_compose_file(&temp_dir), "docker-compose.yml");
}

#[test]
fn detect_default_branch_should_use_current_git_branch() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-init-detect-branch-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");

    Command::new("git")
        .args(["init", "-b", "develop"])
        .current_dir(&temp_dir)
        .output()
        .expect("git init should succeed");

    assert_eq!(detect_default_branch(&temp_dir), "develop");
}
