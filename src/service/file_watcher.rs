use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};

use crate::config::{Config, Server as ConfigServer};
use crate::mc::ban::{self, BannedIps};
use crate::mc::{server_properties, whitelist};
use crate::server::Server;

/// File watcher debounce time.
const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);

/// Service to watch server file changes.
pub fn service(config: Arc<Config>, server: Arc<Server>) {
    // Ensure server directory is set, it must exist
    let dir = match ConfigServer::server_directory(&config) {
        Some(dir) if dir.is_dir() => dir,
        _ => {
            warn!(target: "lazymc", "Server directory doesn't exist, can't watch file changes to reload whitelist and banned IPs");
            return;
        }
    };

    // Keep watching
    #[allow(clippy::blocks_in_conditions)]
    while {
        // Update all files once
        reload_bans(&config, &server, &dir.join(ban::FILE));
        reload_whitelist(&config, &server, &dir);

        // Watch for changes, update accordingly
        watch_server(&config, &server, &dir)
    } {}
}

/// Watch server directory.
///
/// Returns `true` if we should watch again.
#[must_use]
fn watch_server(config: &Config, server: &Server, dir: &Path) -> bool {
    // Directory must exist
    if !dir.is_dir() {
        error!(target: "lazymc", "Server directory does not exist at {} anymore, not watching changes", dir.display());
        return false;
    }

    // Create watcher for directory
    let (tx, rx) = channel();
    let mut watcher =
        watcher(tx, WATCH_DEBOUNCE).expect("failed to create watcher for banned-ips.json");
    if let Err(err) = watcher.watch(dir, RecursiveMode::NonRecursive) {
        error!(target: "lazymc", "An error occured while creating watcher for server files: {}", err);
        return true;
    }

    // Handle change events
    loop {
        match rx.recv().unwrap() {
            // Handle file updates
            DebouncedEvent::Create(ref path)
            | DebouncedEvent::Write(ref path)
            | DebouncedEvent::Remove(ref path) => {
                update(config, server, dir, path);
            }

            // Handle file updates on both paths for rename
            DebouncedEvent::Rename(ref before_path, ref after_path) => {
                update(config, server, dir, before_path);
                update(config, server, dir, after_path);
            }

            // Ignore write/remove notices, will receive write/remove event later
            DebouncedEvent::NoticeWrite(_) | DebouncedEvent::NoticeRemove(_) => {}

            // Ignore chmod changes
            DebouncedEvent::Chmod(_) => {}

            // Rewatch on rescan
            DebouncedEvent::Rescan => {
                debug!(target: "lazymc", "Rescanning server directory files due to file watching problem");
                return true;
            }

            // Rewatch on error
            DebouncedEvent::Error(err, _) => {
                error!(target: "lazymc", "Error occurred while watching server directory for file changes: {}", err);
                return true;
            }
        }
    }
}

/// Process a file change on the given path.
///
/// Should be called both when created, changed or removed.
fn update(config: &Config, server: &Server, dir: &Path, path: &Path) {
    // Update bans
    if path.ends_with(ban::FILE) {
        reload_bans(config, server, path);
    }

    // Update whitelist
    if path.ends_with(whitelist::WHITELIST_FILE)
        || path.ends_with(whitelist::OPS_FILE)
        || path.ends_with(server_properties::FILE)
    {
        reload_whitelist(config, server, dir);
    }
}

/// Reload banned IPs.
fn reload_bans(config: &Config, server: &Server, path: &Path) {
    // Bans must be enabled
    if !config.server.block_banned_ips && !config.server.drop_banned_ips {
        return;
    }

    trace!(target: "lazymc", "Reloading banned IPs...");

    // File must exist, clear file otherwise
    if !path.is_file() {
        debug!(target: "lazymc", "No banned IPs, {} does not exist", ban::FILE);
        // warn!(target: "lazymc", "Not blocking banned IPs, {} file does not exist", ban::FILE);
        server.set_banned_ips_blocking(BannedIps::default());
        return;
    }

    // Load and update banned IPs
    match ban::load(path) {
        Ok(ips) => server.set_banned_ips_blocking(ips),
        Err(err) => {
            debug!(target: "lazymc", "Failed load banned IPs from {}, ignoring: {}", ban::FILE, err);
        }
    }

    // Show warning if 127.0.0.1 is banned
    if server.is_banned_ip_blocking(&("127.0.0.1".parse().unwrap())) {
        warn!(target: "lazymc", "Local address 127.0.0.1 IP banned, probably not what you want");
        warn!(target: "lazymc", "Use '/pardon-ip 127.0.0.1' on the server to unban");
    }
}

/// Reload whitelisted users.
fn reload_whitelist(config: &Config, server: &Server, dir: &Path) {
    // Whitelist must be enabled
    if !config.server.wake_whitelist {
        return;
    }

    // Must be enabled in server.properties
    let enabled = server_properties::read_property(dir.join(server_properties::FILE), "white-list")
        .map(|v| v.trim() == "true")
        .unwrap_or(false);
    if !enabled {
        server.set_whitelist_blocking(None);
        debug!(target: "lazymc", "Not using whitelist, not enabled in {}", server_properties::FILE);
        return;
    }

    trace!(target: "lazymc", "Reloading whitelisted users...");

    // Load and update whitelisted users
    match whitelist::load_dir(dir) {
        Ok(whitelist) => server.set_whitelist_blocking(Some(whitelist)),
        Err(err) => {
            debug!(target: "lazymc", "Failed load whitelist from {}, ignoring: {}", dir.display(), err);
        }
    }
}
