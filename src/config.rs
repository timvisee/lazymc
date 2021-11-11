use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use serde::Deserialize;

use crate::proto;
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
    #[serde(default)]
    pub public: Public,

    /// Server configuration.
    pub server: Server,

    /// Time configuration.
    #[serde(default)]
    pub time: Time,

    /// Messages, shown to the user.
    #[serde(default)]
    pub messages: Messages,

    /// RCON configuration.
    #[serde(default)]
    pub rcon: Rcon,

    /// Advanced configuration.
    #[serde(default)]
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
#[serde(default)]
pub struct Public {
    /// Public address.
    pub address: SocketAddr,

    /// Minecraft protocol version name hint.
    pub version: String,

    /// Minecraft protocol version hint.
    pub protocol: u32,
}

impl Default for Public {
    fn default() -> Self {
        Self {
            address: "0.0.0.0:25565".parse().unwrap(),
            version: proto::PROTO_DEFAULT_VERSION.to_string(),
            protocol: proto::PROTO_DEFAULT_PROTOCOL,
        }
    }
}

/// Server configuration.
#[derive(Debug, Deserialize)]
pub struct Server {
    /// Server directory.
    #[serde(default = "option_pathbuf_dot")]
    pub directory: Option<PathBuf>,

    /// Start command.
    pub command: String,

    /// Server address.
    #[serde(default = "server_address_default")]
    pub address: SocketAddr,

    /// Immediately wake server when starting lazymc.
    #[serde(default)]
    pub wake_on_start: bool,

    /// Immediately wake server after crash.
    #[serde(default)]
    pub wake_on_crash: bool,
}

/// Time configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Time {
    /// Sleep after number of seconds.
    pub sleep_after: u32,

    /// Minimum time in seconds to stay online when server is started.
    #[serde(default, alias = "minimum_online_time")]
    pub min_online_time: u32,

    /// Hold client for number of seconds while server starts, instead of kicking immediately.
    pub hold_client_for: u32,

    /// Server starting timeout. Force kill server process if it takes longer.
    #[serde(alias = "starting_timeout")]
    pub start_timeout: u32,

    /// Server stopping timeout. Force kill server process if it takes longer.
    #[serde(alias = "stopping_timeout")]
    pub stop_timeout: u32,
}

impl Time {
    /// Whether to hold clients.
    pub fn hold(&self) -> bool {
        self.hold_client_for > 0
    }
}

impl Default for Time {
    fn default() -> Self {
        Self {
            sleep_after: 60,
            min_online_time: 60,
            hold_client_for: 25,
            start_timeout: 300,
            stop_timeout: 150,
        }
    }
}

/// Message configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Messages {
    /// MOTD when server is sleeping.
    pub motd_sleeping: String,

    /// MOTD when server is starting.
    pub motd_starting: String,

    /// MOTD when server is stopping.
    pub motd_stopping: String,

    /// Login message when server is starting.
    pub login_starting: String,

    /// Login message when server is stopping.
    pub login_stopping: String,
}

impl Default for Messages {
    fn default() -> Self {
        Self {
            motd_sleeping: "☠ Server is sleeping\n§2☻ Join to start it up".into(),
            motd_starting: "§2☻ Server is starting...\n§7⌛ Please wait...".into(),
            motd_stopping: "☠ Server going to sleep...\n⌛ Please wait...".into(),
            login_starting: "Server is starting... §c♥§r\n\nThis may take some time.\n\nPlease try to reconnect in a minute.".into(),
            login_stopping: "Server is going to sleep... §7☠§r\n\nPlease try to reconnect in a minute to wake it again.".into(),
        }
    }
}

/// RCON configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Rcon {
    /// Enable sleeping server through RCON.
    pub enabled: bool,

    /// Server RCON port.
    pub port: u16,

    /// Server RCON password.
    pub password: String,

    /// Randomize server RCON password on each start.
    pub randomize_password: bool,
}

impl Default for Rcon {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 25575,
            password: "".into(),
            randomize_password: true,
        }
    }
}

/// Advanced configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Advanced {
    /// Rewrite server.properties.
    pub rewrite_server_properties: bool,
}

impl Default for Advanced {
    fn default() -> Self {
        Self {
            rewrite_server_properties: true,
        }
    }
}

fn option_pathbuf_dot() -> Option<PathBuf> {
    Some(".".into())
}

fn server_address_default() -> SocketAddr {
    "127.0.0.1:25566".parse().unwrap()
}
