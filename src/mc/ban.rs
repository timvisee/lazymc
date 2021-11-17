use std::error::Error;
use std::fs;
use std::net::IpAddr;
use std::path::Path;

use serde::Deserialize;

/// File name.
pub const FILE: &str = "banned-ips.json";

/// A banned IP entry.
#[derive(Debug, Deserialize)]
pub struct BannedIp {
    /// Banned IP.
    pub ip: IpAddr,

    /// Ban creation time.
    pub created: String,

    /// Ban source.
    pub source: String,

    /// Ban expiry time.
    pub expires: String,

    /// Ban reason.
    pub reason: String,
}

/// Load banned IPs from file.
pub fn load(path: &Path) -> Result<Vec<BannedIp>, Box<dyn Error>> {
    // Load file contents
    let contents = fs::read_to_string(path)?;

    // Parse contents
    Ok(serde_json::from_str(&contents)?)
}
