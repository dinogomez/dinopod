#![forbid(unsafe_code)]

//! Core library for the `dinopod` command-line tool.

pub mod app;
pub mod cli;
pub mod cmd;
pub mod compose;
pub mod config;
pub mod domain;
pub mod errors;
pub mod fs;
pub mod git;
pub mod lifecycle;
pub mod lock;
pub mod names;
pub mod preflight;
pub mod proxy;
pub mod routes;
pub mod runtime;
pub mod state;
pub mod ui;
