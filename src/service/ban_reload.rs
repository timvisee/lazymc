use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};

use crate::config::Config;
use crate::mc::ban;
use crate::server::Server;

/// File debounce time.
const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);

/// Service to reload banned IPs when its file changes.
pub fn service(config: Arc<Config>, server: Arc<Server>) {
    // TODO: check what happens when file doesn't exist at first?

    // Ensure we need to reload banned IPs
    if !config.server.block_banned_ips && !config.server.drop_banned_ips {
        return;
    }

    // Ensure server directory is set, it must exist
    let dir = match &config.server.directory {
        Some(dir) => dir,
        None => {
            warn!(target: "lazymc", "Not blocking banned IPs, server directory not configured, unable to find {} file", ban::FILE);
            return;
        }
    };

    // Determine file path, ensure it exists
    let path = dir.join(crate::mc::ban::FILE);
    if !path.is_file() {
        warn!(target: "lazymc", "Not blocking banned IPs, {} file does not exist", ban::FILE);
        return;
    }

    // Load banned IPs once
    match ban::load(&path) {
        Ok(ips) => server.set_banned_ips_blocking(ips),
        Err(err) => {
            error!(target: "lazymc", "Failed to load banned IPs from {}: {}", ban::FILE, err);
        }
    }

    // Keep watching
    while watch(&server, &path) {}
}

/// Watch the given file.
fn watch(server: &Server, path: &Path) -> bool {
    // The file must exist
    if !path.is_file() {
        warn!(target: "lazymc", "File {} does not exist, not watching changes", ban::FILE);
        return false;
    }

    // Create watcher for banned IPs file
    let (tx, rx) = channel();
    let mut watcher =
        watcher(tx, WATCH_DEBOUNCE).expect("failed to create watcher for banned-ips.json");
    if let Err(err) = watcher.watch(path, RecursiveMode::NonRecursive) {
        error!(target: "lazymc", "An error occured while creating watcher for {}: {}", ban::FILE, err);
        return true;
    }

    loop {
        // Take next event
        let event = rx.recv().unwrap();

        // Decide whether to reload and rewatch
        let (reload, rewatch) = match event {
            // Reload on write
            DebouncedEvent::NoticeWrite(_) | DebouncedEvent::Write(_) => (true, false),

            // Reload and rewatch on rename/remove
            DebouncedEvent::NoticeRemove(_)
            | DebouncedEvent::Remove(_)
            | DebouncedEvent::Rename(_, _)
            | DebouncedEvent::Rescan
            | DebouncedEvent::Create(_) => {
                trace!(target: "lazymc", "File banned-ips.json removed, trying to rewatch after 1 second");
                thread::sleep(WATCH_DEBOUNCE);
                (true, true)
            }

            // Ignore chmod changes
            DebouncedEvent::Chmod(_) => (false, false),

            // Rewatch on error
            DebouncedEvent::Error(_, _) => (false, true),
        };

        // Reload banned IPs
        if reload {
            info!(target: "lazymc", "Reloading list of banned IPs...");
            match ban::load(&path) {
                Ok(ips) => server.set_banned_ips_blocking(ips),
                Err(err) => {
                    error!(target: "lazymc", "Failed reload list of banned IPs from {}: {}", ban::FILE, err);
                }
            }
        }

        // Rewatch
        if rewatch {
            return true;
        }
    }
}
