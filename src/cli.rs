//! Command-line argument definitions for Dinopod.

use clap::{Parser, Subcommand};

/// Top-level Dinopod command-line interface.
#[derive(Debug, Parser)]
#[command(name = "dinopod")]
#[command(about = "Create isolated per-ticket local development environments")]
pub struct Cli {
    /// Command to execute.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Dinopod subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a starter dinopod.toml in the current repository.
    Init,
    /// Create or reuse a ticket environment.
    Dev {
        /// Ticket or branch identifier.
        ticket: String,
    },
    /// List tracked Dinopod environments.
    List {
        /// Reconcile cached status with Docker and persist updates.
        #[arg(long)]
        reconcile: bool,
    },
    /// Stop an environment while keeping containers and volumes.
    Stop {
        /// Ticket or branch identifier.
        ticket: String,
    },
    /// Stop and remove an environment's containers and networks.
    Down {
        /// Ticket or branch identifier.
        ticket: String,
        /// Remove Compose volumes as well.
        #[arg(long)]
        volumes: bool,
    },
    /// Remove an environment and its worktree.
    Rm {
        /// Ticket or branch identifier.
        ticket: String,
        /// Skip confirmation.
        #[arg(long)]
        yes: bool,
    },
}
