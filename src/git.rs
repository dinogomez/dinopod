//! Git repository and worktree lifecycle planning.

use std::io;
use std::path::{Path, PathBuf};

use crate::cmd::{git_command_failed, path_display, CommandOutput, CommandRunner, CommandSpec};
use crate::errors::{DinopodError, Result};

/// Filesystem checks used by Git worktree orchestration.
pub trait WorktreeFs {
    /// Returns true when `path` exists.
    fn path_exists(&self, path: &Path) -> bool;
}

/// Production filesystem probe for worktree path checks.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdWorktreeFs;

impl WorktreeFs for StdWorktreeFs {
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// Input required to create or reuse a Git worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeRequest {
    repo_root: PathBuf,
    worktree_path: PathBuf,
    branch: String,
    default_branch: String,
}

impl WorktreeRequest {
    /// Creates a worktree request from validated path and branch inputs.
    #[must_use]
    pub fn new(
        repo_root: impl Into<PathBuf>,
        worktree_path: impl Into<PathBuf>,
        branch: impl Into<String>,
        default_branch: impl Into<String>,
    ) -> Self {
        Self {
            repo_root: repo_root.into(),
            worktree_path: worktree_path.into(),
            branch: branch.into(),
            default_branch: default_branch.into(),
        }
    }

    /// Returns the repository root used for Git commands.
    #[must_use]
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Returns the target worktree path.
    #[must_use]
    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    /// Returns the ticket branch name.
    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Returns the configured default branch.
    #[must_use]
    pub fn default_branch(&self) -> &str {
        &self.default_branch
    }
}

/// Result of ensuring a Git worktree exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorktreeAction {
    /// An existing matching worktree was reused.
    Reused,
    /// A worktree was created.
    Created,
}

/// Coordinates Git worktree commands through a command runner.
#[derive(Debug)]
pub struct GitWorktreeManager<'a, R, F> {
    runner: &'a R,
    fs: F,
}

impl<'a, R, F> GitWorktreeManager<'a, R, F>
where
    R: CommandRunner,
    F: WorktreeFs,
{
    /// Creates a Git worktree manager.
    #[must_use]
    pub fn new(runner: &'a R, fs: F) -> Self {
        Self { runner, fs }
    }

    /// Creates or reuses the requested worktree.
    ///
    /// # Errors
    ///
    /// Returns a Git command error when Git fails unexpectedly, or
    /// [`DinopodError::WorktreePathConflict`] when the target path exists but is not
    /// the expected worktree.
    pub fn ensure_worktree(&self, request: &WorktreeRequest) -> Result<WorktreeAction> {
        let entries = self.list_worktrees(request.repo_root())?;

        if entries.iter().any(|entry| entry.matches(request)) {
            return Ok(WorktreeAction::Reused);
        }

        if self.fs.path_exists(request.worktree_path()) {
            return Err(DinopodError::WorktreePathConflict {
                path: request.worktree_path().to_path_buf(),
            });
        }

        if self.branch_exists(request)? {
            self.add_existing_branch_worktree(request)?;
        } else {
            self.add_new_branch_worktree(request)?;
        }

        Ok(WorktreeAction::Created)
    }

    /// Resolves the primary worktree from Git's porcelain worktree list.
    ///
    /// # Errors
    ///
    /// Returns a Git command error when `git worktree list` fails, or
    /// [`DinopodError::GitWorktreeRootUnavailable`] when no worktree is reported.
    pub fn resolve_primary_worktree(&self, current_dir: &Path) -> Result<PathBuf> {
        self.list_worktrees(current_dir)?
            .into_iter()
            .next()
            .map(|entry| entry.path)
            .ok_or(DinopodError::GitWorktreeRootUnavailable)
    }

    fn list_worktrees(&self, current_dir: &Path) -> Result<Vec<WorktreeEntry>> {
        let args = ["worktree", "list", "--porcelain"];
        let output = self.run_git(current_dir, args)?;
        Ok(parse_worktree_list(output.stdout()))
    }

    fn branch_exists(&self, request: &WorktreeRequest) -> Result<bool> {
        let args = vec![
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            format!("refs/heads/{}", request.branch()),
        ];
        let output = self.run_git_allow_failure(request.repo_root(), args)?;
        Ok(output.success())
    }

    fn add_existing_branch_worktree(&self, request: &WorktreeRequest) -> Result<()> {
        let args = [
            "worktree".to_owned(),
            "add".to_owned(),
            path_display(request.worktree_path()),
            request.branch().to_owned(),
        ];
        self.run_git(request.repo_root(), args)?;
        Ok(())
    }

    fn add_new_branch_worktree(&self, request: &WorktreeRequest) -> Result<()> {
        let args = [
            "worktree".to_owned(),
            "add".to_owned(),
            "-b".to_owned(),
            request.branch().to_owned(),
            path_display(request.worktree_path()),
            request.default_branch().to_owned(),
        ];
        self.run_git(request.repo_root(), args)?;
        Ok(())
    }

    fn run_git<I, S>(&self, current_dir: &Path, args: I) -> Result<CommandOutput>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let output = self.run_git_allow_failure(current_dir, args.clone())?;
        if output.success() {
            Ok(output)
        } else {
            Err(git_command_failed(args, &output))
        }
    }

    fn run_git_allow_failure(
        &self,
        current_dir: &Path,
        args: Vec<String>,
    ) -> io::Result<CommandOutput> {
        let command = CommandSpec::new("git")
            .args(args)
            .current_dir(current_dir.to_path_buf());
        self.runner.run(&command)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorktreeEntry {
    path: PathBuf,
    branch: Option<String>,
}

impl WorktreeEntry {
    fn matches(&self, request: &WorktreeRequest) -> bool {
        paths_match(&self.path, request.worktree_path())
            && self.branch.as_deref() == Some(request.branch())
    }
}

fn paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    let Ok(left) = left.canonicalize() else {
        return false;
    };
    let Ok(right) = right.canonicalize() else {
        return false;
    };

    left == right
}

fn parse_worktree_list(input: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current_path = None;
    let mut current_branch = None;

    for line in input.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(path) = current_path.replace(PathBuf::from(path)) {
                entries.push(WorktreeEntry {
                    path,
                    branch: current_branch.take(),
                });
            }
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch.to_owned());
        }
    }

    if let Some(path) = current_path {
        entries.push(WorktreeEntry {
            path,
            branch: current_branch,
        });
    }

    entries
}
