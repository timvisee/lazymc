use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::net::IpAddr;
use std::path::Path;

use serde::Deserialize;

/// File name.
pub const FILE: &str = "banned-ips.json";

/// List of banned IPs.
#[derive(Debug, Default)]
pub struct BannedIps {
    /// List of banned IPs.
    ips: HashMap<IpAddr, BannedIp>,
}

impl BannedIps {
    /// Get ban entry if IP if it exists.
    ///
    /// This uses the latest known `banned-ips.json` contents if known.
    /// If this feature is disabled, this will always return false.
    pub fn get(&self, ip: &IpAddr) -> Option<BannedIp> {
        self.ips.get(ip).cloned()
    }

    /// Check whether the given IP is banned.
    ///
    /// This uses the latest known `banned-ips.json` contents if known.
    /// If this feature is disabled, this will always return false.
    pub fn is_banned(&self, ip: &IpAddr) -> bool {
        self.ips.get(ip).map(|ip| ip.is_banned()).unwrap_or(false)
    }
}

/// A banned IP entry.
#[derive(Debug, Deserialize, Clone)]
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

impl BannedIp {
    /// Check if this entry is currently banned.
    pub fn is_banned(&self) -> bool {
        // TODO: check expiry date here!
        true
    }
}

/// Load banned IPs from file.
pub fn load(path: &Path) -> Result<BannedIps, Box<dyn Error>> {
    // Load file contents
    let contents = fs::read_to_string(path)?;

    // Parse contents, transform into map
    let ips: Vec<BannedIp> = serde_json::from_str(&contents)?;
    let ips = ips.into_iter().map(|ip| (ip.ip, ip)).collect();

    Ok(BannedIps { ips })
}
