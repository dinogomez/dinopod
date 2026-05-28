//! Terminal output boundary for user-facing messages.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Writes user-facing command output.
pub trait Ui {
    /// Writes an in-progress status line.
    fn status(&mut self, message: &str) -> io::Result<()>;

    /// Writes a successful result line.
    fn success(&mut self, message: &str) -> io::Result<()>;

    /// Writes a warning line.
    fn warning(&mut self, message: &str) -> io::Result<()>;

    /// Writes an error line.
    fn error(&mut self, message: &str) -> io::Result<()>;

    /// Advances lifecycle output: completes the previous step, starts a new one.
    fn step_progress(&mut self, message: &str) -> io::Result<()> {
        self.status(message)
    }

    /// Marks the active lifecycle step failed.
    fn step_fail(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Marks the active lifecycle step successful and clears step state.
    fn step_finalize(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Prefixes lifecycle steps with spinners/checkmarks and the pod id after the glyph.
pub struct EnvironmentUi<U: Ui> {
    id: String,
    inner: U,
    color: bool,
    tty: bool,
    active_step: Option<String>,
    spinner: Option<SpinnerController>,
}

const SUMMARY_GLYPH: &str = "🦕";

impl<U: Ui> EnvironmentUi<U> {
    /// Creates an environment-scoped UI wrapper.
    #[must_use]
    pub fn new(id: impl Into<String>, inner: U) -> Self {
        Self {
            id: id.into(),
            inner,
            color: use_color(),
            tty: io::stdout().is_terminal(),
            active_step: None,
            spinner: None,
        }
    }
}

impl<U: Ui> Drop for EnvironmentUi<U> {
    fn drop(&mut self) {
        if let Some(spinner) = self.spinner.take() {
            spinner.stop();
        }
    }
}

impl<U: Ui> Ui for EnvironmentUi<U> {
    fn status(&mut self, message: &str) -> io::Result<()> {
        self.step_progress(message)
    }

    fn success(&mut self, message: &str) -> io::Result<()> {
        self.write_summary_line(message)
    }

    fn warning(&mut self, message: &str) -> io::Result<()> {
        let line = format_environment_line(
            StepGlyph::WARN,
            Style::YELLOW,
            &self.id,
            message,
            self.color,
        );
        self.inner.warning(&line)
    }

    fn error(&mut self, message: &str) -> io::Result<()> {
        let line = format_environment_line(
            StepGlyph::FAIL,
            Style::RED,
            &self.id,
            message,
            self.color,
        );
        self.inner.error(&line)
    }

    fn step_progress(&mut self, message: &str) -> io::Result<()> {
        if let Some(previous) = self.active_step.take() {
            self.render_step(&previous, StepState::Success)?;
        }
        self.active_step = Some(message.to_owned());
        self.render_step(message, StepState::Running)
    }

    fn step_fail(&mut self) -> io::Result<()> {
        if let Some(previous) = self.active_step.take() {
            self.render_step(&previous, StepState::Failed)?;
        }
        Ok(())
    }

    fn step_finalize(&mut self) -> io::Result<()> {
        if let Some(previous) = self.active_step.take() {
            self.render_step(&previous, StepState::Success)?;
        }
        Ok(())
    }
}

impl<U: Ui> EnvironmentUi<U> {
    fn write_summary_line(&mut self, message: &str) -> io::Result<()> {
        let line = format_environment_line(
            StepGlyph::OK,
            Style::GREEN,
            &self.id,
            message,
            self.color,
        );
        self.inner.success(&line)
    }

    /// Prints a one-line start marker before a foreground passthrough command.
    pub fn command_start(&mut self, command: &str) -> io::Result<()> {
        let message = format!("running `{command}`");
        let line = format_environment_line("▶", Style::CYAN, &self.id, &message, self.color);
        let mut out = io::stdout().lock();
        writeln!(out, "{line}")?;
        out.flush()
    }

    /// Prints a blank line before the post-run summary block.
    pub fn summary_header(&mut self) -> io::Result<()> {
        writeln!(io::stdout())
    }

    /// Prints a summary label/value row prefixed with a dinosaur marker.
    pub fn summary_row(&mut self, label: &str, value: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        if self.color {
            writeln!(
                out,
                "  {SUMMARY_GLYPH} {}{}{}  {}{}",
                Style::DIM,
                label,
                Style::RESET,
                value,
                Style::RESET,
            )?;
        } else {
            writeln!(out, "  {SUMMARY_GLYPH} {label}  {value}")?;
        }
        out.flush()
    }

    /// Prints a dim next-step hint using this environment's id.
    pub fn summary_next_command(&mut self) -> io::Result<()> {
        self.summary_action(&format!("dinopod {} <command>", self.id))
    }

    /// Prints a dim hint line in the summary block.
    pub fn summary_action(&mut self, message: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        if self.color {
            writeln!(
                out,
                "  {SUMMARY_GLYPH} {}{}{}",
                Style::DIM,
                message,
                Style::RESET,
            )?;
        } else {
            writeln!(out, "  {SUMMARY_GLYPH} {message}")?;
        }
        out.flush()
    }

    /// Prints a dim logs hint using this environment's id.
    pub fn summary_logs_follow(&mut self) -> io::Result<()> {
        self.summary_action(&format!("dinopod logs {} -f", self.id))
    }

    fn render_step(&mut self, message: &str, state: StepState) -> io::Result<()> {
        if let Some(spinner) = self.spinner.take() {
            spinner.stop();
        }

        if self.tty && state == StepState::Running {
            self.spinner = Some(SpinnerController::start(
                message.to_owned(),
                self.id.clone(),
                self.color,
            ));
            return Ok(());
        }

        let glyph = state.glyph(self.tty);
        let glyph_style = match state {
            StepState::Running => Style::CYAN,
            StepState::Success => Style::GREEN,
            StepState::Failed => Style::RED,
        };
        let line = format_environment_line(glyph, glyph_style, &self.id, message, self.color);
        write_step_line(&line, self.tty, state != StepState::Running)
    }
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

/// Prints the Dinopod welcome banner (dinosaur + wordmark inline).
///
/// # Errors
///
/// Returns an I/O error when stdout cannot be written.
pub fn print_banner() -> io::Result<()> {
    let color = use_color();
    let mut out = io::stdout().lock();
    write_inline_banner(&mut out, color)?;
    writeln!(out)?;
    Ok(())
}

/// Formats a list row with status first, then pod id, project, and URL.
#[must_use]
pub fn format_list_record(record: &crate::state::EnvironmentRecord) -> String {
    let status = list_status_label(&record.status);
    if !use_color() {
        return format!(
            "{status}\t[{}]\t{}\t{}",
            record.ticket, record.project, record.url
        );
    }

    let status_style = list_status_style(&record.status);
    format!(
        "{}{}{}  {}{}[{}]{}  {}  {}{}{}",
        status_style,
        status,
        Style::RESET,
        Style::BOLD,
        Style::CYAN,
        record.ticket,
        Style::RESET,
        record.project,
        Style::GREEN,
        record.url,
        Style::RESET,
    )
}

fn list_status_label(status: &crate::state::EnvironmentStatus) -> &'static str {
    match status {
        crate::state::EnvironmentStatus::Running => "Running",
        crate::state::EnvironmentStatus::Stopped => "Stopped",
        crate::state::EnvironmentStatus::Down => "Down",
        crate::state::EnvironmentStatus::Stale => "Stale",
    }
}

fn list_status_style(status: &crate::state::EnvironmentStatus) -> &'static str {
    match status {
        crate::state::EnvironmentStatus::Running => Style::GREEN,
        crate::state::EnvironmentStatus::Stopped | crate::state::EnvironmentStatus::Down => {
            Style::DIM
        }
        crate::state::EnvironmentStatus::Stale => Style::YELLOW,
    }
}

/// Prompts on stderr when stdin is an interactive terminal.
///
/// Returns `None` when stdin is not a TTY.
///
/// # Errors
///
/// Returns an I/O error when the prompt or response cannot be read.
pub fn prompt_yes_no(prompt: &str) -> io::Result<Option<bool>> {
    use std::io::{BufRead, IsTerminal};

    if !io::stdin().is_terminal() {
        return Ok(None);
    }

    if use_color() {
        write!(
            io::stderr().lock(),
            "{}{prompt}{} [y/N] ",
            Style::YELLOW,
            Style::RESET
        )?;
    } else {
        write!(io::stderr().lock(), "{prompt} [y/N] ")?;
    }
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(Some(matches!(answer.as_str(), "y" | "yes")))
}

fn use_color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

struct Style;

impl Style {
    const RESET: &'static str = "\x1b[0m";
    const BOLD: &'static str = "\x1b[1m";
    const DIM: &'static str = "\x1b[2m";
    const GREEN: &'static str = "\x1b[32m";
    const CYAN: &'static str = "\x1b[36m";
    const YELLOW: &'static str = "\x1b[33m";
    const RED: &'static str = "\x1b[31m";
}

struct StepGlyph;

impl StepGlyph {
    const OK: &'static str = "✓";
    const FAIL: &'static str = "✗";
    const WARN: &'static str = "!";
    const RUNNING_TTY: &'static str = "⠋";
    const RUNNING_PLAIN: &'static str = "...";
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepState {
    Running,
    Success,
    Failed,
}

impl StepState {
    fn glyph(self, tty: bool) -> &'static str {
        match self {
            Self::Running if tty => StepGlyph::RUNNING_TTY,
            Self::Running => StepGlyph::RUNNING_PLAIN,
            Self::Success => StepGlyph::OK,
            Self::Failed => StepGlyph::FAIL,
        }
    }
}

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct SpinnerLine {
    message: String,
    id: String,
    color: bool,
}

struct SpinnerController {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
    _keep_alive: (Arc<AtomicUsize>, Arc<Mutex<SpinnerLine>>),
}

impl SpinnerController {
    fn start(message: String, id: String, color: bool) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let frame = Arc::new(AtomicUsize::new(0));
        let line = Arc::new(Mutex::new(SpinnerLine { message, id, color }));
        let stop_clone = Arc::clone(&stop);
        let frame_clone = Arc::clone(&frame);
        let line_clone = Arc::clone(&line);
        let handle = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                let idx = frame_clone.load(Ordering::Relaxed) % SPINNER_FRAMES.len();
                let glyph = SPINNER_FRAMES[idx];
                if let Ok(line) = line_clone.lock() {
                    let rendered = format_environment_line(
                        glyph,
                        Style::CYAN,
                        &line.id,
                        &line.message,
                        line.color,
                    );
                    let _ = write_step_line(&rendered, true, false);
                }
                frame_clone.fetch_add(1, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(80));
            }
        });
        Self {
            stop,
            handle,
            _keep_alive: (frame, line),
        }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

fn format_environment_line(
    glyph: &str,
    glyph_style: &str,
    id: &str,
    message: &str,
    color: bool,
) -> String {
    if color {
        format!(
            "{}{}{} {}{}{}  {}{}",
            glyph_style,
            glyph,
            Style::RESET,
            Style::BOLD,
            Style::CYAN,
            id,
            Style::RESET,
            message,
        )
    } else {
        format!("{glyph} {id}  {message}")
    }
}

fn write_step_line(line: &str, tty: bool, newline: bool) -> io::Result<()> {
    let mut out = io::stdout().lock();
    if tty {
        if newline {
            write!(out, "\r\x1b[2K{line}\n")?;
        } else {
            write!(out, "\r\x1b[2K{line}")?;
        }
    } else {
        writeln!(out, "{line}")?;
    }
    out.flush()
}

/// Prints the init wizard subtitle beneath the banner.
///
/// # Errors
///
/// Returns an I/O error when stderr cannot be written.
pub fn print_init_subtitle() -> io::Result<()> {
    let color = use_color();
    let mut err = io::stderr().lock();
    if color {
        writeln!(
            err,
            "{}{}  Configure dinopod.toml for this repository.{}{}",
            Style::DIM,
            Style::RESET,
            Style::DIM,
            Style::RESET,
        )?;
    } else {
        writeln!(err, "  Configure dinopod.toml for this repository.")?;
    }
    writeln!(err)?;
    err.flush()
}

/// Prompts for a single init wizard value with an optional detection hint.
///
/// # Errors
///
/// Returns an I/O error when the prompt or response cannot be read.
pub fn init_prompt(label: &str, hint: Option<&str>, default: &str) -> io::Result<String> {
    let color = use_color();
    let mut err = io::stderr().lock();
    if color {
        writeln!(
            err,
            "{}{}◇{}  {}{}{}",
            Style::CYAN,
            Style::RESET,
            Style::RESET,
            Style::BOLD,
            label,
            Style::RESET,
        )?;
        if let Some(hint) = hint {
            writeln!(
                err,
                "     {}{}{}",
                Style::DIM,
                hint,
                Style::RESET,
            )?;
        }
        write!(
            err,
            "     {}{}{} {}›{} ",
            Style::CYAN,
            default,
            Style::RESET,
            Style::DIM,
            Style::RESET,
        )?;
    } else {
        writeln!(err, "?  {label}")?;
        if let Some(hint) = hint {
            writeln!(err, "   {hint}")?;
        }
        write!(err, "   {default} › ")?;
    }
    err.flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Prints a concise success message after `dinopod init` writes config.
///
/// # Errors
///
/// Returns an I/O error when stdout cannot be written.
pub fn print_init_complete(path: &Path) -> io::Result<()> {
    let color = use_color();
    let mut out = io::stdout().lock();
    writeln!(out)?;
    if color {
        writeln!(
            out,
            "{}{}✓{} Created {}{}{}",
            Style::GREEN,
            Style::RESET,
            Style::RESET,
            Style::BOLD,
            path.display(),
            Style::RESET,
        )?;
        writeln!(out)?;
        writeln!(
            out,
            "  {}dinopod new{} {}<id>{}",
            Style::DIM,
            Style::RESET,
            Style::CYAN,
            Style::RESET,
        )?;
    } else {
        writeln!(out, "Created {}", path.display())?;
        writeln!(out)?;
        writeln!(out, "  dinopod new <id>")?;
    }
    out.flush()
}

/// Block-letter "dinopod" wordmark (figlet-style).
const BANNER_WORDMARK: [&str; 7] = [
    "░███████   ░██████░███    ░██   ░██████   ░█████████    ░██████   ░███████   ",
    "░██   ░██    ░██  ░████   ░██  ░██   ░██  ░██     ░██  ░██   ░██  ░██   ░██  ",
    "░██    ░██   ░██  ░██░██  ░██ ░██     ░██ ░██     ░██ ░██     ░██ ░██    ░██ ",
    "░██    ░██   ░██  ░██ ░██ ░██ ░██     ░██ ░█████████  ░██     ░██ ░██    ░██ ",
    "░██    ░██   ░██  ░██  ░██░██ ░██     ░██ ░██         ░██     ░██ ░██    ░██ ",
    "░██   ░██    ░██  ░██   ░████  ░██   ░██  ░██          ░██   ░██  ░██   ░██  ",
    "░███████   ░██████░██    ░███   ░██████   ░██           ░██████   ░███████   ",
];

fn write_inline_banner(out: &mut impl Write, color: bool) -> io::Result<()> {
    for wordmark in BANNER_WORDMARK {
        if color {
            writeln!(
                out,
                "{}{}{wordmark}{}",
                Style::BOLD,
                Style::GREEN,
                Style::RESET
            )?;
        } else {
            writeln!(out, "{wordmark}")?;
        }
    }

    if color {
        writeln!(
            out,
            "{}{}{}",
            Style::DIM,
            crate::cli::ABOUT,
            Style::RESET
        )?;
    } else {
        writeln!(out, "{}", crate::cli::ABOUT)?;
    }
    Ok(())
}

/// Prints bytes as lines prefixed with `[id]`.
///
/// # Errors
///
/// Returns an I/O error when stdout/stderr cannot be written.
pub fn print_prefixed_lines(id: &str, bytes: &[u8], to_stderr: bool) -> io::Result<()> {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        if to_stderr {
            writeln!(io::stderr(), "[{id}] {line}")?;
        } else {
            writeln!(io::stdout(), "[{id}] {line}")?;
        }
    }
    io::stderr().flush()?;
    io::stdout().flush()?;
    Ok(())
}

pub(crate) fn lifecycle_progress(ui: &mut Option<&mut dyn Ui>, message: &str) {
    if let Some(ui) = ui.as_mut() {
        let _ = ui.step_progress(message);
    }
}

pub(crate) fn lifecycle_fail(ui: &mut Option<&mut dyn Ui>) {
    if let Some(ui) = ui.as_mut() {
        let _ = ui.step_fail();
    }
}

pub(crate) fn lifecycle_finalize(ui: &mut Option<&mut dyn Ui>) {
    if let Some(ui) = ui.as_mut() {
        let _ = ui.step_finalize();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn environment_line_should_place_id_after_glyph() {
        let line = format_environment_line(
            StepGlyph::OK,
            Style::GREEN,
            "branch-33",
            "provisioning pod",
            false,
        );

        assert_eq!(line, "✓ branch-33  provisioning pod");
    }

    #[test]
    fn list_record_should_put_status_first() {
        let record = crate::state::EnvironmentRecord {
            project: "prompt-enhancer-branch-33".to_owned(),
            ticket: "branch-33".to_owned(),
            host: "branch-33-prompt-enhancer.localhost".to_owned(),
            url: "http://branch-33-prompt-enhancer.localhost".to_owned(),
            worktree_path: PathBuf::from("/tmp/worktree"),
            route_path: PathBuf::from("/tmp/route.yml"),
            user_compose_path: None,
            compose_override_path: None,
            status: crate::state::EnvironmentStatus::Running,
            runtime_mode: None,
            dev_script: None,
            app_host_port: None,
            env_overlay_path: None,
            port_plan: None,
        };

        let line = format_list_record(&record);

        assert!(line.starts_with("Running\t[branch-33]\t"));
    }
}
