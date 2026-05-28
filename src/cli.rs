//! Command-line argument definitions for Dinopod.

use clap::{Parser, Subcommand};

/// One-line description for help text and the welcome banner.
pub const ABOUT: &str = "Worktrees with isolated docker and routing.";

/// Top-level Dinopod command-line interface.
#[derive(Debug, Parser)]
#[command(name = "dinopod")]
#[command(version)]
#[command(about = ABOUT)]
#[command(after_help = r#"Examples:
  dinopod init
  dinopod init -y
  dinopod new number-1
  dinopod number-1 pnpm dev:all
  dinopod stop number-1
  dinopod rm number-1 --yes
"#)]
pub struct Cli {
    /// Command to execute.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Dinopod subcommands.
///
/// Lifecycle verbs use `dinopod <command> <id>`. Arbitrary worktree commands use
/// `dinopod <id> <command...>` (rewritten internally to the hidden `run` subcommand).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create dinopod.toml (interactive wizard, or defaults with --yes).
    Init {
        /// Write defaults without prompting.
        #[arg(short, long)]
        yes: bool,
    },
    /// Provision a pod: worktree, isolated compose, and setup commands.
    New {
        /// Pod ID / worktree slug (e.g. number-1).
        #[arg(value_name = "ID")]
        id: String,
    },
    /// List tracked pods.
    List {
        /// Reconcile cached status with Docker and persist updates.
        #[arg(long)]
        reconcile: bool,
    },
    /// Stop containers for a pod while keeping volumes.
    Stop {
        /// Pod ID.
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Stop and remove containers for a pod (and optionally volumes).
    Down {
        /// Pod ID.
        #[arg(value_name = "ID")]
        id: String,
        /// Remove Compose volumes as well.
        #[arg(long)]
        volumes: bool,
    },
    /// Remove a pod and its worktree.
    Rm {
        /// Pod ID.
        #[arg(value_name = "ID")]
        id: String,
        /// Skip the interactive confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Tail the native dev log file when present.
    Logs {
        /// Pod ID.
        #[arg(value_name = "ID")]
        id: String,
        /// Follow log output.
        #[arg(short, long)]
        follow: bool,
    },
    /// `dinopod <id> <command>` (inserted by [`rewrite_argv`] in `main`).
    #[command(hide = true, name = "run")]
    Run {
        /// Pod ID.
        #[arg(value_name = "ID")]
        id: String,
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            num_args = 1..
        )]
        command: Vec<String>,
    },
    /// Hidden alias for `dinopod <id> <command>`.
    #[command(hide = true)]
    Exec {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            num_args = 1..
        )]
        command: Vec<String>,
    },
    /// Deprecated: use `dinopod new <ID>`.
    #[command(hide = true)]
    Dev {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(long)]
        script: Option<String>,
        #[arg(long)]
        refresh_env: bool,
        #[arg(long)]
        no_install: bool,
        #[arg(long)]
        detach: bool,
    },
    /// Deprecated: use `dinopod new <ID>` then `dinopod <ID> pnpm dev:all`.
    #[command(hide = true, name = "dev:all")]
    DevAll {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(long)]
        refresh_env: bool,
        #[arg(long)]
        no_install: bool,
        #[arg(long)]
        detach: bool,
    },
}

/// Rewrites `dinopod <id> <command...>` into `dinopod run <id> <command...>`.
#[must_use]
pub fn rewrite_argv(argv: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(program) = argv.first() else {
        return argv.to_vec();
    };
    out.push(program.clone());
    let rest: Vec<String> = argv.iter().skip(1).cloned().collect();
    if rest.is_empty() || is_global_command(&rest[0]) {
        out.extend(rest);
        return out;
    }
    out.push("run".to_owned());
    out.extend(rest);
    out
}

fn is_global_command(word: &str) -> bool {
    matches!(
        word,
        "init"
            | "new"
            | "list"
            | "stop"
            | "down"
            | "rm"
            | "logs"
            | "run"
            | "exec"
            | "dev"
            | "dev:all"
            | "help"
    ) || word.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::rewrite_argv;

    #[test]
    fn rewrite_should_insert_run_for_id_first_commands() {
        let argv = vec![
            "dinopod".to_owned(),
            "pro-2000".to_owned(),
            "pnpm".to_owned(),
            "dev:all".to_owned(),
        ];
        assert_eq!(
            rewrite_argv(&argv),
            vec![
                "dinopod".to_owned(),
                "run".to_owned(),
                "pro-2000".to_owned(),
                "pnpm".to_owned(),
                "dev:all".to_owned(),
            ]
        );
    }

    #[test]
    fn rewrite_should_leave_lifecycle_verbs_untouched() {
        let argv = vec![
            "dinopod".to_owned(),
            "rm".to_owned(),
            "pro-2000".to_owned(),
            "--yes".to_owned(),
        ];
        assert_eq!(
            rewrite_argv(&argv),
            vec![
                "dinopod".to_owned(),
                "rm".to_owned(),
                "pro-2000".to_owned(),
                "--yes".to_owned(),
            ]
        );
    }

    #[test]
    fn rewrite_should_leave_new_untouched() {
        let argv = vec![
            "dinopod".to_owned(),
            "new".to_owned(),
            "ticket-1".to_owned(),
        ];
        assert_eq!(
            rewrite_argv(&argv),
            vec![
                "dinopod".to_owned(),
                "new".to_owned(),
                "ticket-1".to_owned(),
            ]
        );
    }

    #[test]
    fn parse_run_should_capture_script_names_with_colons() {
        use clap::Parser;

        let argv = rewrite_argv(&[
            "dinopod".to_owned(),
            "branch-34".to_owned(),
            "pnpm".to_owned(),
            "dev:all".to_owned(),
        ]);
        let cli = super::Cli::try_parse_from(argv).expect("argv should parse");
        match cli.command {
            Some(super::Command::Run { id, command }) => {
                assert_eq!(id, "branch-34");
                assert_eq!(command, vec!["pnpm".to_owned(), "dev:all".to_owned()]);
            }
            other => panic!("expected run command, got {other:?}"),
        }
    }
}
