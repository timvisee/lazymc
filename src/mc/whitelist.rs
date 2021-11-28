use std::error::Error;
use std::fs;
use std::path::Path;

use serde::Deserialize;

/// Whitelist file name.
pub const WHITELIST_FILE: &str = "whitelist.json";

/// OPs file name.
pub const OPS_FILE: &str = "ops.json";

/// Whitelisted users.
///
/// Includes list of OPs, which are also automatically whitelisted.
#[derive(Debug, Default)]
pub struct Whitelist {
    /// Whitelisted users.
    whitelist: Vec<String>,

    /// OPd users.
    ops: Vec<String>,
}

impl Whitelist {
    /// Check whether a user is whitelisted.
    pub fn is_whitelisted(&self, username: &str) -> bool {
        self.whitelist.iter().any(|u| u == username) || self.ops.iter().any(|u| u == username)
    }
}

/// A whitelist user.
#[derive(Debug, Deserialize, Clone)]
pub struct WhitelistUser {
    /// Whitelisted username.
    #[serde(rename = "name", alias = "username")]
    pub username: String,

    /// Whitelisted UUID.
    pub uuid: Option<String>,
}

/// An OP user.
#[derive(Debug, Deserialize, Clone)]
pub struct OpUser {
    /// OP username.
    #[serde(rename = "name", alias = "username")]
    pub username: String,

    /// OP UUID.
    pub uuid: Option<String>,

    /// OP level.
    pub level: Option<u32>,

    /// Whether OP can bypass player limit.
    #[serde(rename = "bypassesPlayerLimit")]
    pub byapsses_player_limit: Option<bool>,
}

/// Load whitelist from directory.
pub fn load_dir(path: &Path) -> Result<Whitelist, Box<dyn Error>> {
    let whitelist_file = path.join(WHITELIST_FILE);
    let ops_file = path.join(OPS_FILE);

    // Load whitelist users
    let whitelist = if whitelist_file.is_file() {
        load_whitelist(&whitelist_file)?
    } else {
        vec![]
    };

    // Load OPd users
    let ops = if ops_file.is_file() {
        load_ops(&ops_file)?
    } else {
        vec![]
    };

    debug!(target: "lazymc", "Loaded {} whitelist and {} OP users", whitelist.len(), ops.len());

    Ok(Whitelist { whitelist, ops })
}

/// Load whitelist from file.
fn load_whitelist(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    // Load file contents
    let contents = fs::read_to_string(path)?;

    // Parse contents
    let users: Vec<WhitelistUser> = serde_json::from_str(&contents)?;

    // Pluck usernames
    Ok(users.into_iter().map(|user| user.username).collect())
}

/// Load OPs from file.
fn load_ops(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    // Load file contents
    let contents = fs::read_to_string(path)?;

    // Parse contents
    let users: Vec<OpUser> = serde_json::from_str(&contents)?;

    // Pluck usernames
    Ok(users.into_iter().map(|user| user.username).collect())
}
