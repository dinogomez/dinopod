//! Command execution boundary for Git, Docker, and Compose adapters.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

/// A fully described external command invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
    current_dir: Option<PathBuf>,
    env: Vec<(String, String)>,
}

impl CommandSpec {
    /// Creates a command spec for `program`.
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            current_dir: None,
            env: Vec::new(),
        }
    }

    /// Adds a command argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds multiple command arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets the command working directory.
    #[must_use]
    pub fn current_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(path.into());
        self
    }

    /// Adds an environment variable override.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Returns the executable name or path.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Returns the command arguments.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.args
    }

    /// Returns the configured working directory.
    #[must_use]
    pub fn working_dir(&self) -> Option<&Path> {
        self.current_dir.as_deref()
    }

    /// Returns configured environment overrides.
    #[must_use]
    pub fn environment(&self) -> &[(String, String)] {
        &self.env
    }
}

/// Captured output from an external command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CommandOutput {
    /// Creates command output from explicit parts.
    #[must_use]
    pub fn new(success: bool, exit_code: Option<i32>, stdout: String, stderr: String) -> Self {
        Self {
            success,
            exit_code,
            stdout,
            stderr,
        }
    }

    /// Creates successful command output for tests and fake adapters.
    #[must_use]
    pub fn successful(stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self::new(true, Some(0), stdout.into(), stderr.into())
    }

    /// Creates failed command output for tests and fake adapters.
    #[must_use]
    pub fn failed(
        exit_code: Option<i32>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::new(false, exit_code, stdout.into(), stderr.into())
    }

    /// Returns true when the command exited successfully.
    #[must_use]
    pub fn success(&self) -> bool {
        self.success
    }

    /// Returns the process exit code when available.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// Returns captured standard output.
    #[must_use]
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Returns captured standard error.
    #[must_use]
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
}

/// Runs external commands.
pub trait CommandRunner {
    /// Executes `command` and captures its output.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the process cannot be spawned or its output cannot be read.
    fn run(&self, command: &CommandSpec) -> io::Result<CommandOutput>;
}

/// Production command runner backed by [`std::process::Command`].
#[derive(Clone, Copy, Debug, Default)]
pub struct StdCommandRunner;

impl CommandRunner for StdCommandRunner {
    fn run(&self, command: &CommandSpec) -> io::Result<CommandOutput> {
        let mut process = ProcessCommand::new(command.program());
        process.args(command.arguments());
        process.envs(
            command
                .environment()
                .iter()
                .map(|(key, value)| (key, value)),
        );

        if let Some(current_dir) = command.working_dir() {
            process.current_dir(current_dir);
        }

        let output = process.output()?;
        Ok(CommandOutput::new(
            output.status.success(),
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}
