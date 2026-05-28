#![forbid(unsafe_code)]

//! Core library for the `dinopod` command-line tool.

pub mod app;
pub mod cli;
pub mod cmd;
pub mod compose;
pub mod config;
pub mod detect;
pub mod domain;
pub mod env;
pub mod errors;
pub mod fs;
pub mod git;
pub mod init_wizard;
pub mod lifecycle;
pub mod lock;
pub mod names;
pub mod preflight;
pub mod process;
pub mod proxy;
pub mod routes;
pub mod runtime;
pub mod state;
pub mod ui;
