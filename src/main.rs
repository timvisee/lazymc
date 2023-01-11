#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate derive_builder;
#[macro_use]
extern crate log;

pub(crate) mod action;
pub(crate) mod cli;
pub(crate) mod config;
pub(crate) mod forge;
pub(crate) mod join;
#[cfg(feature = "lobby")]
pub(crate) mod lobby;
pub(crate) mod mc;
pub(crate) mod monitor;
pub(crate) mod net;
pub(crate) mod os;
pub(crate) mod probe;
pub(crate) mod proto;
pub(crate) mod proxy;
pub(crate) mod server;
pub(crate) mod service;
pub(crate) mod status;
pub(crate) mod types;
pub(crate) mod util;

use std::env;

use clap::Command;

// Compile time feature compatability check.
#[cfg(all(windows, not(feature = "rcon")))]
compile_error!("Must enable \"rcon\" feature on Windows.");

/// Default log level if none is set.
const LOG_DEFAULT: &str = "info";

/// Main entrypoint.
fn main() -> Result<(), ()> {
    // Initialize logger
    init_log();

    // Build clap app, invoke intended action
    let app = cli::app();
    invoke_action(app)
}

/// Initialize logger.
fn init_log() {
    // Load .env variables
    let _ = dotenv::dotenv();

    // Set default log level if none is set
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", LOG_DEFAULT);
    }

    // Initialize logger
    pretty_env_logger::init();
}

/// Invoke an action.
fn invoke_action(app: Command) -> Result<(), ()> {
    let matches = app.get_matches();

    // Config operations
    if let Some(matches) = matches.subcommand_matches("config") {
        if let Some(matches) = matches.subcommand_matches("generate") {
            action::config_generate::invoke(matches);
            return Ok(());
        }

        if let Some(matches) = matches.subcommand_matches("test") {
            action::config_test::invoke(matches);
            return Ok(());
        }

        unreachable!();
    }

    // Start server
    action::start::invoke(&matches)
}
