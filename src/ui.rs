//! Terminal output boundary for user-facing messages.

use std::io::{self, Write};

/// Writes user-facing command output.
pub trait Ui {
    /// Writes an informational status line.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the output sink cannot be written.
    fn status(&mut self, message: &str) -> io::Result<()>;

    /// Writes a successful result line.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the output sink cannot be written.
    fn success(&mut self, message: &str) -> io::Result<()>;

    /// Writes a warning line.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the output sink cannot be written.
    fn warning(&mut self, message: &str) -> io::Result<()>;

    /// Writes an error line.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the output sink cannot be written.
    fn error(&mut self, message: &str) -> io::Result<()>;
}

/// In-memory UI sink for tests and core orchestration checks.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BufferedUi {
    stdout: Vec<String>,
    stderr: Vec<String>,
}

impl BufferedUi {
    /// Returns captured standard output lines.
    #[must_use]
    pub fn stdout(&self) -> &[String] {
        &self.stdout
    }

    /// Returns captured standard error lines.
    #[must_use]
    pub fn stderr(&self) -> &[String] {
        &self.stderr
    }
}

impl Ui for BufferedUi {
    fn status(&mut self, message: &str) -> io::Result<()> {
        self.stdout.push(message.to_owned());
        Ok(())
    }

    fn success(&mut self, message: &str) -> io::Result<()> {
        self.stdout.push(message.to_owned());
        Ok(())
    }

    fn warning(&mut self, message: &str) -> io::Result<()> {
        self.stderr.push(message.to_owned());
        Ok(())
    }

    fn error(&mut self, message: &str) -> io::Result<()> {
        self.stderr.push(message.to_owned());
        Ok(())
    }
}

/// UI sink backed by process stdout and stderr.
#[derive(Debug, Default)]
pub struct TerminalUi;

impl Ui for TerminalUi {
    fn status(&mut self, message: &str) -> io::Result<()> {
        writeln!(io::stdout().lock(), "{message}")
    }

    fn success(&mut self, message: &str) -> io::Result<()> {
        writeln!(io::stdout().lock(), "{message}")
    }

    fn warning(&mut self, message: &str) -> io::Result<()> {
        writeln!(io::stderr().lock(), "{message}")
    }

    fn error(&mut self, message: &str) -> io::Result<()> {
        writeln!(io::stderr().lock(), "{message}")
    }
}
