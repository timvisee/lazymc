use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use serde::Deserialize;
use version_compare::Cmp;

use crate::proto;
use crate::util::error::{quit_error, quit_error_msg, ErrorHintsBuilder};

/// Default configuration file location.
pub const CONFIG_FILE: &str = "lazymc.toml";

/// Configuration version user should be using, or warning will be shown.
const CONFIG_VERSION: &str = "0.2.0";

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

    /// MOTD configuration.
    #[serde(default)]
    pub motd: Motd,

    /// Join configuration.
    #[serde(default)]
    pub join: Join,

    /// Join kick configuration.
    #[serde(default)]
    pub join_kick: JoinKick,

    /// Join hold configuration.
    #[serde(default)]
    pub join_hold: JoinHold,

    /// Lockout feature.
    #[serde(default)]
    pub lockout: Lockout,

    /// RCON configuration.
    #[serde(default)]
    pub rcon: Rcon,

    /// Advanced configuration.
    #[serde(default)]
    pub advanced: Advanced,

    /// Config configuration.
    #[serde(default)]
    pub config: ConfigConfig,
}

impl Config {
    /// Load configuration from file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, io::Error> {
        let data = fs::read(path)?;
        let config: Config = toml::from_slice(&data)?;

        // Show warning if config version is problematic
        match &config.config.version {
            None => warn!(target: "lazymc::config", "Config version unknown, it may be outdated"),
            Some(version) => match version_compare::compare_to(version, CONFIG_VERSION, Cmp::Ge) {
                Ok(false) => {
                    warn!(target: "lazymc::config", "Config is for older lazymc version, you may need to update it")
                }
                Err(_) => {
                    warn!(target: "lazymc::config", "Config version is invalid, you may need to update it")
                }
                Ok(true) => {}
            },
        }

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

    /// Server starting timeout. Force kill server process if it takes longer.
    #[serde(default = "u32_300")]
    pub start_timeout: u32,

    /// Server stopping timeout. Force kill server process if it takes longer.
    #[serde(default = "u32_150")]
    pub stop_timeout: u32,
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
}

impl Default for Time {
    fn default() -> Self {
        Self {
            sleep_after: 60,
            min_online_time: 60,
        }
    }
}

/// MOTD configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Motd {
    /// MOTD when server is sleeping.
    pub sleeping: String,

    /// MOTD when server is starting.
    pub starting: String,

    /// MOTD when server is stopping.
    pub stopping: String,

    /// Use MOTD from Minecraft server once known.
    pub from_server: bool,
}

impl Default for Motd {
    fn default() -> Self {
        Self {
            sleeping: "☠ Server is sleeping\n§2☻ Join to start it up".into(),
            starting: "§2☻ Server is starting...\n§7⌛ Please wait...".into(),
            stopping: "☠ Server going to sleep...\n⌛ Please wait...".into(),
            from_server: false,
        }
    }
}

/// Join method types.
#[derive(Debug, Deserialize, Copy, Clone, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Hold,
    Kick,
}

/// Join configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Join {
    /// Join methods.
    pub methods: Vec<Method>,
}

impl Default for Join {
    fn default() -> Self {
        Self {
            methods: vec![Method::Hold, Method::Kick],
        }
    }
}

/// Join kick configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct JoinKick {
    /// Kick message when server is starting.
    pub starting: String,

    /// Kick message when server is stopping.
    pub stopping: String,
}

impl Default for JoinKick {
    fn default() -> Self {
        Self {
            starting: "Server is starting... §c♥§r\n\nThis may take some time.\n\nPlease try to reconnect in a minute.".into(),
            stopping: "Server is going to sleep... §7☠§r\n\nPlease try to reconnect in a minute to wake it again.".into(),
        }
    }
}

/// Join hold configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct JoinHold {
    /// Hold client for number of seconds on connect while server starts.
    pub timeout: u32,
}

impl JoinHold {
    /// Whether to hold clients.
    // TODO: remove this
    pub fn hold(&self) -> bool {
        self.timeout > 0
    }
}

impl Default for JoinHold {
    fn default() -> Self {
        Self { timeout: 25 }
    }
}

/// Lockout configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Lockout {
    /// Enable to prevent everybody from connecting through lazymc. Instantly kicks player.
    pub enabled: bool,

    /// Kick players with following message.
    pub message: String,
}

impl Default for Lockout {
    fn default() -> Self {
        Self {
            enabled: false,
            message: "Server is closed §7☠§r\n\nPlease come back another time.".into(),
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

/// Config configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ConfigConfig {
    /// Configuration for lazymc version.
    pub version: Option<String>,
}

impl Default for ConfigConfig {
    fn default() -> Self {
        Self { version: None }
    }
}

fn option_pathbuf_dot() -> Option<PathBuf> {
    Some(".".into())
}

fn server_address_default() -> SocketAddr {
    "127.0.0.1:25566".parse().unwrap()
}

fn u32_300() -> u32 {
    300
}

fn u32_150() -> u32 {
    300
}
