#![allow(unknown_lints)]
#![allow(
    clippy::collapsible_if,
    clippy::manual_is_multiple_of,
    clippy::io_other_error
)]

pub mod app;
pub mod config;
pub mod demo;
pub mod discovery;
pub mod history;
pub mod hooks;
pub mod logger;
pub mod models;
pub mod monitor;
pub mod orchestrator;
pub mod process;
pub mod recorder;
pub mod session;
pub mod session_recorder;
pub mod terminals;
pub mod theme;
pub mod transcript;
pub mod ui;
