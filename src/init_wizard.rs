//! Interactive `dinopod init` wizard.

use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{render_starter_config, DinopodConfig};
use crate::errors::{DinopodError, Result};
use crate::ui::{init_prompt, print_banner, print_init_subtitle};

const COMPOSE_CANDIDATES: &[&str] = &[
    "docker-compose.yml",
    "compose.yml",
    "docker-compose.yaml",
    "compose.yaml",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BranchSource {
    CurrentBranch,
    OriginHead,
    LocalRef,
    Fallback,
}

/// Builds configuration for `dinopod init`, applying repo-aware defaults when possible.
#[must_use]
pub fn default_init_config() -> DinopodConfig {
    detected_init_config(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Applies compose-file and default-branch detection on top of documented defaults.
#[must_use]
pub fn detected_init_config(cwd: &Path) -> DinopodConfig {
    let mut config = DinopodConfig::default();
    let compose_file = detect_compose_file(cwd);
    config.compose.file = compose_file.clone().into();
    config.app.compose_file = compose_file.into();
    let (default_branch, _) = detect_default_branch_with_source(cwd);
    config.git.default_branch.clone_from(&default_branch);
    config.app.default_branch = default_branch;
    config
}

/// Runs the interactive init wizard and returns rendered `dinopod.toml` contents.
///
/// # Errors
///
/// Returns a recoverable error when prompts cannot be read.
pub fn run_init_wizard() -> Result<String> {
    let cwd = std::env::current_dir().map_err(DinopodError::Io)?;
    if !io::stdin().is_terminal() {
        return Ok(render_starter_config(&detected_init_config(&cwd)));
    }

    print_banner()?;
    print_init_subtitle()?;

    let mut config = detected_init_config(&cwd);
    let compose_default = config.compose.file.display().to_string();
    let compose_hint = compose_file_hint(&cwd, &compose_default);
    let compose_file = init_prompt("Compose file", Some(&compose_hint), &compose_default)
        .map_err(DinopodError::Io)?;
    config.compose.file = compose_file.clone().into();
    config.app.compose_file = compose_file.into();

    let (branch_default, branch_source) = detect_default_branch_with_source(&cwd);
    let branch_hint = branch_source.hint();
    let default_branch = init_prompt(
        "Default branch for new pods",
        Some(branch_hint),
        &branch_default,
    )
    .map_err(DinopodError::Io)?;
    config.git.default_branch.clone_from(&default_branch);
    config.app.default_branch = default_branch;

    let rendered = render_starter_config(&config);
    DinopodConfig::from_toml_str(&rendered).map_err(DinopodError::Config)?;
    Ok(rendered)
}

/// Returns the first compose file found in `cwd`, or the conventional default.
#[must_use]
pub fn detect_compose_file(cwd: &Path) -> String {
    for candidate in COMPOSE_CANDIDATES {
        if cwd.join(candidate).is_file() {
            return (*candidate).to_owned();
        }
    }
    "docker-compose.yml".to_owned()
}

/// Returns the best default branch for new pods in `cwd`.
#[must_use]
pub fn detect_default_branch(cwd: &Path) -> String {
    detect_default_branch_with_source(cwd).0
}

fn detect_default_branch_with_source(cwd: &Path) -> (String, BranchSource) {
    if let Some(branch) = git_output(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        if !branch.is_empty() && branch != "HEAD" {
            return (branch, BranchSource::CurrentBranch);
        }
    }

    if let Some(origin) = git_output(
        cwd,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        if let Some(branch) = origin.strip_prefix("origin/") {
            if !branch.is_empty() {
                return (branch.to_owned(), BranchSource::OriginHead);
            }
        }
    }

    for candidate in ["main", "master"] {
        if git_succeeds(
            cwd,
            &["show-ref", "--verify", "--quiet", &format!("refs/heads/{candidate}")],
        ) {
            return (candidate.to_owned(), BranchSource::LocalRef);
        }
    }

    ("main".to_owned(), BranchSource::Fallback)
}

fn compose_file_hint(cwd: &Path, file: &str) -> String {
    if cwd.join(file).is_file() {
        "Found in project root".to_owned()
    } else {
        "No compose file detected; using default".to_owned()
    }
}

impl BranchSource {
    fn hint(self) -> &'static str {
        match self {
            Self::CurrentBranch => "Current git branch",
            Self::OriginHead => "From origin/HEAD",
            Self::LocalRef => "Local branch exists in this repo",
            Self::Fallback => "Default when git metadata is unavailable",
        }
    }
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn git_succeeds(cwd: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .is_ok_and(|status| status.success())
}
