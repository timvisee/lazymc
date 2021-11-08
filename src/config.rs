use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Default configuration file location.
pub const CONFIG_FILE: &str = "lazymc.toml";

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
}

impl Config {
    /// Load configuration form file.
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
    pub directory: PathBuf,

    /// Start command.
    pub command: String,

    /// Ingress address.
    #[serde(alias = "address_ingress")]
    pub address: SocketAddr,
}

/// Time configuration.
#[derive(Debug, Deserialize)]
pub struct Time {
    /// Sleep after number of seconds.
    pub sleep_after: u32,

    /// Minimum time in seconds to stay online when server is started.
    // TODO: implement this
    #[serde(alias = "minimum_online_time")]
    pub min_online_time: u32,
}

/// Messages.
#[derive(Debug, Deserialize)]
pub struct Messages {
    /// MOTD when server is sleeping.
    pub motd_sleeping: String,

    /// MOTD when server is starting.
    pub motd_starting: String,

    /// Login message when server is starting.
    pub login_starting: String,
}
