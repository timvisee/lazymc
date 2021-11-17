use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::net::IpAddr;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// File name.
pub const FILE: &str = "banned-ips.json";

/// The forever expiry literal.
const EXPIRY_FOREVER: &str = "forever";

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
    pub created: Option<String>,

    /// Ban source.
    pub source: Option<String>,

    /// Ban expiry time.
    pub expires: Option<String>,

    /// Ban reason.
    pub reason: Option<String>,
}

impl BannedIp {
    /// Check if this entry is currently banned.
    pub fn is_banned(&self) -> bool {
        // Get expiry time
        let expires = match &self.expires {
            Some(expires) => expires,
            None => return true,
        };

        // If expiry is forever, the user is banned
        if expires.trim().to_lowercase() == EXPIRY_FOREVER {
            return true;
        }

        // Parse expiry time, check if it has passed
        let expiry = match DateTime::parse_from_str(expires, "%Y-%m-%d %H:%M:%S %z") {
            Ok(expiry) => expiry,
            Err(err) => {
                error!(target: "lazymc", "Failed to parse ban expiry '{}', assuming still banned: {}", expires, err);
                return true;
            }
        };

        expiry > Utc::now()
    }
}

/// Load banned IPs from file.
pub fn load(path: &Path) -> Result<BannedIps, Box<dyn Error>> {
    // Load file contents
    let contents = fs::read_to_string(path)?;

    // Parse contents
    let ips: Vec<BannedIp> = serde_json::from_str(&contents)?;
    debug!(target: "lazymc", "Loaded {} banned IPs", ips.len());

    // Transform into map
    let ips = ips.into_iter().map(|ip| (ip.ip, ip)).collect();
    Ok(BannedIps { ips })
}
