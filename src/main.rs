#![forbid(unsafe_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use dinopod::app::AppContext;
use dinopod::cli::{Cli, Command};
use dinopod::config::{render_starter_config, DinopodConfig};
use dinopod::errors::{DinopodError, Result};
use dinopod::ui::{TerminalUi, Ui};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dinopod: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let Some(command) = cli.command else {
        let mut command = Cli::command();
        command.print_help()?;
        println!();
        return Ok(());
    };

    match command {
        Command::Init => init(Path::new("dinopod.toml")),
        Command::Dev { ticket } => {
            let context = AppContext::for_dev(&std::env::current_dir()?)?;
            let manager = context.lifecycle_manager();
            let mut ui = TerminalUi;
            let summary = manager.dev(&ticket)?;
            for warning in &summary.warnings {
                ui.warning(&warning.to_string())?;
            }
            ui.success(&format!("worktree: {}", summary.worktree_path.display()))?;
            ui.success(&format!("project: {}", summary.project))?;
            ui.success(&format!("url: {}", summary.url))?;
            Ok(())
        }
        Command::List { reconcile } => {
            let context = AppContext::for_list(&std::env::current_dir()?)?;
            let manager = context.lifecycle_manager();
            let mut ui = TerminalUi;
            let records = if reconcile {
                manager.list_reconciled()?
            } else {
                manager.list()?
            };
            for record in records {
                ui.status(&format!(
                    "{}\t{:?}\t{}",
                    record.project, record.status, record.url
                ))?;
            }
            Ok(())
        }
        Command::Stop { ticket } => {
            let context = AppContext::for_mutating(&std::env::current_dir()?, false)?;
            context.lifecycle_manager().stop(&ticket)
        }
        Command::Down { ticket, volumes } => {
            let context = AppContext::for_mutating(&std::env::current_dir()?, false)?;
            context.lifecycle_manager().down(&ticket, volumes)
        }
        Command::Rm { ticket, yes } => {
            let context = AppContext::for_mutating(&std::env::current_dir()?, false)?;
            context.lifecycle_manager().rm(&ticket, yes)
        }
    }
}

fn init(path: &Path) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                DinopodError::ConfigAlreadyExists {
                    path: path.to_path_buf(),
                }
            } else {
                DinopodError::from(error)
            }
        })?;
    file.write_all(render_starter_config(&DinopodConfig::default()).as_bytes())?;
    Ok(())
}
