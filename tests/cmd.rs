use std::cell::RefCell;
use std::io;
use std::path::Path;

use dinopod::cmd::{CommandOutput, CommandRunner, CommandSpec};

#[derive(Debug, Default)]
struct RecordingRunner {
    commands: RefCell<Vec<CommandSpec>>,
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: &CommandSpec) -> io::Result<CommandOutput> {
        self.commands.borrow_mut().push(command.clone());
        Ok(CommandOutput::successful("clean", ""))
    }
}

fn run_git_status<R>(runner: &R) -> io::Result<CommandOutput>
where
    R: CommandRunner,
{
    let command = CommandSpec::new("git")
        .arg("status")
        .current_dir("/repo")
        .env("GIT_TERMINAL_PROMPT", "0");

    runner.run(&command)
}

#[test]
fn command_runner_boundary_should_capture_command_shape_without_trait_objects() {
    let runner = RecordingRunner::default();
    let output = run_git_status(&runner).expect("fake command should run");
    let commands = runner.commands.borrow();
    let command = commands.first().expect("command should be recorded");

    assert!(output.success());
    assert_eq!(output.stdout(), "clean");
    assert_eq!(command.program(), "git");
    assert_eq!(command.arguments(), ["status"]);
    assert_eq!(command.working_dir(), Some(Path::new("/repo")));
    assert_eq!(
        command.environment(),
        [("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned())]
    );
}
