use std::cell::RefCell;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use dinopod::cmd::{CommandOutput, CommandRunner, CommandSpec};
use dinopod::errors::DinopodError;
use dinopod::git::{GitWorktreeManager, WorktreeAction, WorktreeFs, WorktreeRequest};

#[derive(Debug, Default)]
struct FakeRunner {
    outputs: RefCell<Vec<CommandOutput>>,
    commands: RefCell<Vec<CommandSpec>>,
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
        Ok(self.outputs.borrow_mut().remove(0))
    }
}

#[derive(Debug, Default)]
struct FakeFs {
    existing_paths: HashSet<PathBuf>,
}

impl WorktreeFs for FakeFs {
    fn path_exists(&self, path: &Path) -> bool {
        self.existing_paths.contains(path)
    }
}

fn request() -> WorktreeRequest {
    WorktreeRequest::new(
        Path::new("/repo/myapp"),
        Path::new("/repo/.dinopod-worktrees/myapp-jira-123"),
        "jira-123",
        "main",
    )
}

#[test]
fn new_ticket_without_branch_should_create_branch_worktree_from_default_branch() {
    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful("", ""));
    runner.push_output(CommandOutput::failed(Some(1), "", "missing branch"));
    runner.push_output(CommandOutput::successful("", ""));
    let manager = GitWorktreeManager::new(&runner, FakeFs::default());

    let action = manager
        .ensure_worktree(&request())
        .expect("new worktree should be created");

    assert_eq!(action, WorktreeAction::Created);
    assert_eq!(
        runner.command_arguments(),
        [
            ["worktree", "list", "--porcelain"]
                .map(String::from)
                .to_vec(),
            ["rev-parse", "--verify", "refs/heads/jira-123",]
                .map(String::from)
                .to_vec(),
            [
                "worktree",
                "add",
                "-b",
                "jira-123",
                "/repo/.dinopod-worktrees/myapp-jira-123",
                "main",
            ]
            .map(String::from)
            .to_vec(),
        ]
    );
}

#[test]
fn existing_local_branch_should_add_worktree_without_creating_branch() {
    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful("", ""));
    runner.push_output(CommandOutput::successful("refs/heads/jira-123\n", ""));
    runner.push_output(CommandOutput::successful("", ""));
    let manager = GitWorktreeManager::new(&runner, FakeFs::default());

    let action = manager
        .ensure_worktree(&request())
        .expect("existing branch should be checked out");

    assert_eq!(action, WorktreeAction::Created);
    assert_eq!(
        runner.command_arguments()[2],
        [
            "worktree",
            "add",
            "/repo/.dinopod-worktrees/myapp-jira-123",
            "jira-123",
        ]
        .map(String::from)
        .to_vec()
    );
}

#[test]
fn existing_matching_worktree_should_be_reused_without_git_mutation() {
    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful(
        "worktree /repo/.dinopod-worktrees/myapp-jira-123\nbranch refs/heads/jira-123\n",
        "",
    ));
    let manager = GitWorktreeManager::new(&runner, FakeFs::default());

    let action = manager
        .ensure_worktree(&request())
        .expect("existing worktree should be reused");

    assert_eq!(action, WorktreeAction::Reused);
    assert_eq!(runner.command_arguments().len(), 1);
}

#[cfg(unix)]
#[test]
fn existing_matching_worktree_should_be_reused_when_git_reports_canonical_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dinopod-worktree-canonical-test-{}",
        std::process::id()
    ));
    let real_root = temp_dir.join("real");
    let link_root = temp_dir.join("link");
    let request_path = link_root.join("myapp-jira-123");
    let canonical_path = real_root.join("myapp-jira-123");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&canonical_path).expect("canonical worktree path should be created");
    symlink(&real_root, &link_root).expect("symlinked root should be created");

    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful(
        format!(
            "worktree {}\nbranch refs/heads/jira-123\n",
            canonical_path.display()
        ),
        "",
    ));
    let fs = FakeFs {
        existing_paths: HashSet::from([request_path.clone()]),
    };
    let manager = GitWorktreeManager::new(&runner, fs);

    let action = manager
        .ensure_worktree(&WorktreeRequest::new(
            Path::new("/repo/myapp"),
            &request_path,
            "jira-123",
            "main",
        ))
        .expect("canonicalized worktree should be reused");

    assert_eq!(action, WorktreeAction::Reused);
}

#[test]
fn existing_non_worktree_path_should_return_conflict() {
    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful("", ""));
    let fs = FakeFs {
        existing_paths: HashSet::from([PathBuf::from("/repo/.dinopod-worktrees/myapp-jira-123")]),
    };
    let manager = GitWorktreeManager::new(&runner, fs);

    let error = manager
        .ensure_worktree(&request())
        .expect_err("unexpected path should block worktree creation");

    assert!(matches!(error, DinopodError::WorktreePathConflict { .. }));
    assert_eq!(runner.command_arguments().len(), 1);
}

#[test]
fn primary_worktree_resolution_should_return_first_git_worktree() {
    let runner = FakeRunner::default();
    runner.push_output(CommandOutput::successful(
        "worktree /repo/myapp\nbranch refs/heads/main\n\nworktree /repo/.dinopod-worktrees/myapp-jira-123\nbranch refs/heads/jira-123\n",
        "",
    ));
    let manager = GitWorktreeManager::new(&runner, FakeFs::default());

    let root = manager
        .resolve_primary_worktree(Path::new("/repo/.dinopod-worktrees/myapp-jira-123"))
        .expect("primary worktree should resolve from porcelain output");

    assert_eq!(root, PathBuf::from("/repo/myapp"));
}
