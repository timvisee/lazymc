use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use serde::Deserialize;

use crate::util::error::{quit_error, quit_error_msg, ErrorHintsBuilder};

/// Default configuration file location.
pub const CONFIG_FILE: &str = "lazymc.toml";

/// Load config from file, based on CLI arguments.
///
/// Quits with an error message on failure.
pub fn load(matches: &ArgMatches) -> Config {
    // Get config path, attempt to canonicalize
    let mut path = PathBuf::from(matches.value_of("config").unwrap());
    if let Ok(p) = path.canonicalize() {
        path = p;
    }

    // Ensure configuration file exists
    if !path.is_file() {
        quit_error_msg(
            format!(
                "Config file does not exist: {}",
                path.to_str().unwrap_or("?")
            ),
            ErrorHintsBuilder::default()
                .config(true)
                .config_generate(true)
                .build()
                .unwrap(),
        );
    }

    // Load config
    let config = match Config::load(path) {
        Ok(config) => config,
        Err(err) => {
            quit_error(
                anyhow!(err).context("Failed to load config"),
                ErrorHintsBuilder::default()
                    .config(true)
                    .config_test(true)
                    .build()
                    .unwrap(),
            );
        }
    };

    config
}

/// Configuration.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Public configuration.
    pub public: Public,

    /// Server configuration.
    pub server: Server,

    /// Time configuration.
    pub time: Time,

    /// Messages, shown to the user.
    pub messages: Messages,

    /// RCON configuration.
    pub rcon: Rcon,

    /// Advanced configuration.
    pub advanced: Advanced,
}

impl Config {
    /// Load configuration from file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, io::Error> {
        let data = fs::read(path)?;
        let config = toml::from_slice(&data)?;
        Ok(config)
    }
}

/// Public configuration.
#[derive(Debug, Deserialize)]
pub struct Public {
    /// Egress address.
    #[serde(alias = "address_egress")]
    pub address: SocketAddr,
}

/// Server configuration.
#[derive(Debug, Deserialize)]
pub struct Server {
    /// Server directory.
    pub directory: Option<PathBuf>,

    /// Start command.
    pub command: String,

    /// Ingress address.
    #[serde(alias = "address_ingress")]
    pub address: SocketAddr,

    /// Immediately wake server when starting lazymc.
    #[serde(default)]
    pub wake_on_start: bool,
}

/// Time configuration.
#[derive(Debug, Deserialize)]
pub struct Time {
    /// Sleep after number of seconds.
    pub sleep_after: u32,

    /// Minimum time in seconds to stay online when server is started.
    #[serde(default, alias = "minimum_online_time")]
    pub min_online_time: u32,
}

/// Message configuration.
#[derive(Debug, Deserialize)]
pub struct Messages {
    /// MOTD when server is sleeping.
    pub motd_sleeping: String,

    /// MOTD when server is starting.
    pub motd_starting: String,

    /// Login message when server is starting.
    pub login_starting: String,
}

/// RCON configuration.
#[derive(Debug, Deserialize)]
pub struct Rcon {
    /// Enable sleeping server through RCON.
    pub enabled: bool,

    /// Server RCON port.
    pub port: u16,

    /// Server RCON password.
    pub password: String,

    /// Randomize ingress server RCON password on each start.
    pub randomize_password: bool,
}

/// Advanced configuration.
#[derive(Debug, Deserialize)]
pub struct Advanced {
    /// Rewrite server.properties.
    pub rewrite_server_properties: bool,
}
