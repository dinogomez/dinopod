#![forbid(unsafe_code)]

use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use dinopod::app::AppContext;
use dinopod::cli::{rewrite_argv, Cli, Command};
use dinopod::config::render_starter_config;
use dinopod::errors::{DinopodError, Result};
use dinopod::git::path_is_within;
use dinopod::init_wizard::{default_init_config, run_init_wizard};
use dinopod::lifecycle::{DevOptions, PodSummary};
use dinopod::process::{exec_in_worktree_foreground, format_exec_command, DevProcessPaths};
use dinopod::state::EnvironmentRecord;
use dinopod::ui::{
    format_list_record, print_banner, print_init_complete, print_prefixed_lines, prompt_yes_no,
    EnvironmentUi, TerminalUi, Ui,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            if io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
                eprintln!("\x1b[31mdinopod:\x1b[0m {error}");
            } else {
                eprintln!("dinopod: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let argv: Vec<String> = std::env::args().collect();
    let cli = Cli::try_parse_from(rewrite_argv(&argv))
        .unwrap_or_else(|error| error.exit());

    let Some(command) = cli.command else {
        print_banner()?;
        Cli::command().about(None).print_help()?;
        println!();
        return Ok(ExitCode::SUCCESS);
    };

    match command {
        Command::Init { yes } => {
            init(Path::new("dinopod.toml"), yes)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::New { id } => {
            run_new(&id)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::List { reconcile } => {
            let context = if reconcile {
                AppContext::for_list_reconcile(&std::env::current_dir()?)?
            } else {
                AppContext::for_list(&std::env::current_dir()?)?
            };
            let manager = context.lifecycle_manager();
            let mut ui = TerminalUi;
            let records = if reconcile {
                manager.list_reconciled()?
            } else {
                manager.list()?
            };
            for record in records {
                ui.status(&format_list_record(&record))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Stop { id } => run_stop(&id),
        Command::Down { id, volumes } => run_down(&id, volumes),
        Command::Rm { id, yes } => run_rm(&id, yes),
        Command::Logs { id, follow } => {
            logs(&id, follow)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Run { id, command } | Command::Exec { id, command } => run_in_pod(&id, &command),
        Command::Dev {
            id,
            script,
            refresh_env,
            no_install,
            detach,
        } => {
            eprintln!(
                "warning: `dinopod dev` is deprecated; use `dinopod new {id}` then `dinopod {id} <command>`"
            );
            let options = DevOptions {
                script,
                refresh_env,
                no_install,
                detach,
            };
            run_dev(&id, &options)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::DevAll {
            id,
            refresh_env,
            no_install,
            detach,
        } => {
            eprintln!(
                "warning: `dinopod dev:all` is deprecated; use `dinopod new {id}` then `dinopod {id} pnpm dev:all`"
            );
            let options = DevOptions {
                script: Some("dev:all".to_owned()),
                refresh_env,
                no_install,
                detach,
            };
            run_dev(&id, &options)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_stop(id: &str) -> Result<ExitCode> {
    let mut ui = EnvironmentUi::new(id, TerminalUi);
    let context = AppContext::for_mutating(&std::env::current_dir()?, false)?;
    context.lifecycle_manager().stop(id, Some(&mut ui))?;
    Ok(ExitCode::SUCCESS)
}

fn run_down(id: &str, volumes: bool) -> Result<ExitCode> {
    let mut ui = EnvironmentUi::new(id, TerminalUi);
    let context = AppContext::for_mutating(&std::env::current_dir()?, false)?;
    context
        .lifecycle_manager()
        .down(id, volumes, Some(&mut ui))?;
    Ok(ExitCode::SUCCESS)
}

fn run_rm(id: &str, yes: bool) -> Result<ExitCode> {
    let mut ui = EnvironmentUi::new(id, TerminalUi);
    let context = AppContext::for_mutating(&std::env::current_dir()?, false)?;
    let manager = context.lifecycle_manager();
    let record = manager.find_record(id)?;
    warn_if_cwd_inside_worktree(&record, context.repo_root(), &mut ui)?;
    if !yes {
        let prompt = format!(
            "Remove pod [{id}] {}?\nWorktree: {}\nThis cannot be undone.",
            record.project,
            record.worktree_path.display()
        );
        match prompt_yes_no(&prompt).map_err(DinopodError::Io)? {
            Some(true) => {}
            Some(false) => {
                eprintln!("Aborted.");
                return Ok(ExitCode::SUCCESS);
            }
            None => {
                return Err(DinopodError::ConfirmationRequired {
                    ticket: id.to_owned(),
                });
            }
        }
    }
    manager.rm(id, true, Some(&mut ui))?;
    print_rm_summary(&mut ui, &record)?;
    Ok(ExitCode::SUCCESS)
}

fn run_new(id: &str) -> Result<()> {
    let mut ui = EnvironmentUi::new(id, TerminalUi);
    let context = AppContext::for_new(&std::env::current_dir()?)?;
    let summary = context.lifecycle_manager().new_pod(id, Some(&mut ui))?;
    print_pod_summary(&mut ui, &summary)?;
    Ok(())
}

fn run_in_pod(id: &str, command: &[String]) -> Result<ExitCode> {
    let Some((program, args)) = command.split_first() else {
        return Err(DinopodError::ExecCommandRequired);
    };

    let context = AppContext::for_run(&std::env::current_dir()?)?;
    let record = context.lifecycle_manager().find_record(id)?;
    let env = context
        .lifecycle_manager()
        .merged_env_for_worktree(&record.worktree_path)?;

    let command_line = format_exec_command(program, args);
    let mut ui = EnvironmentUi::new(id, TerminalUi);
    ui.command_start(&command_line)?;
    let status = exec_in_worktree_foreground(&record.worktree_path, program, args, &env)?;
    Ok(exit_code_from_status(status))
}

fn run_dev(id: &str, options: &DevOptions) -> Result<()> {
    let mut env_ui = EnvironmentUi::new(id, TerminalUi);
    env_ui.status("running preflight checks")?;

    let foreground_launch = {
        let context = AppContext::for_dev(&std::env::current_dir()?)?;
        let manager = context.lifecycle_manager();
        let summary = manager.dev_with_options(id, options, Some(&mut env_ui))?;
        print_dev_summary(&mut env_ui, &summary)?;
        summary.native_dev
    };

    if let Some(launch) = foreground_launch {
        println!();
        dinopod::process::run_dev_process_foreground(&launch)?;
    }
    Ok(())
}

fn warn_if_cwd_inside_worktree(
    record: &EnvironmentRecord,
    repo_root: &Path,
    ui: &mut EnvironmentUi<TerminalUi>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    if !path_is_within(&cwd, &record.worktree_path) {
        return Ok(());
    }

    ui.warning(&format!(
        "current directory is inside the worktree being removed; run from the main repo instead:\n  cd {}\n  dinopod rm {}",
        repo_root.display(),
        record.ticket,
    ))?;
    Ok(())
}

fn exit_code_from_status(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) if (0..=255).contains(&code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Some(_) | None => ExitCode::FAILURE,
    }
}

fn print_rm_summary(ui: &mut EnvironmentUi<TerminalUi>, record: &EnvironmentRecord) -> Result<()> {
    ui.summary_header()?;
    ui.summary_row("removed worktree", &record.worktree_path.display().to_string())?;
    ui.summary_row("removed project", &record.project)?;
    Ok(())
}

fn print_pod_summary(ui: &mut EnvironmentUi<TerminalUi>, summary: &PodSummary) -> Result<()> {
    for warning in &summary.warnings {
        ui.warning(&warning.to_string())?;
    }
    ui.summary_header()?;
    ui.summary_row("worktree", &summary.worktree_path.display().to_string())?;
    ui.summary_row("project", &summary.project)?;
    ui.summary_row("url", &summary.url)?;
    ui.summary_next_command()?;
    Ok(())
}

fn print_dev_summary(
    ui: &mut EnvironmentUi<TerminalUi>,
    summary: &dinopod::lifecycle::DevSummary,
) -> Result<()> {
    for warning in &summary.warnings {
        ui.warning(&warning.to_string())?;
    }
    ui.summary_header()?;
    ui.summary_row("worktree", &summary.worktree_path.display().to_string())?;
    ui.summary_row("project", &summary.project)?;
    ui.summary_row("url", &summary.url)?;
    if let Some(pid) = summary.background_pid {
        ui.summary_row("pid", &pid.to_string())?;
        ui.summary_logs_follow()?;
    }
    Ok(())
}

fn logs(id: &str, follow: bool) -> Result<()> {
    let context = AppContext::for_list(&std::env::current_dir()?)?;
    let record = context.lifecycle_manager().find_record(id)?;
    let log_path = DevProcessPaths::new(&record.worktree_path).log_file;
    if !log_path.is_file() {
        return Err(DinopodError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("dev log not found: {}", log_path.display()),
        )));
    }

    if follow {
        let status = std::process::Command::new("tail")
            .args(["-f", "-n", "200", &log_path.display().to_string()])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(DinopodError::Io(std::io::Error::other("tail failed")))
        }
    } else {
        let contents = std::fs::read_to_string(log_path)?;
        print_prefixed_lines(id, contents.as_bytes(), false)?;
        Ok(())
    }
}

fn init(path: &Path, yes: bool) -> Result<()> {
    if path.exists() {
        return Err(DinopodError::ConfigAlreadyExists {
            path: path.to_path_buf(),
        });
    }

    let contents = if yes {
        render_starter_config(&default_init_config())
    } else {
        run_init_wizard()?
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, contents)?;
    print_init_complete(path)?;
    Ok(())
}
