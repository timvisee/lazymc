use std::collections::HashMap;
use std::sync::Arc;

use clap::ArgMatches;

use crate::config::{self, Config};
use crate::mc::server_properties;
use crate::service;

/// Start lazymc.
pub async fn invoke(matches: &ArgMatches) -> Result<(), ()> {
    // Load config
    let config = Arc::new(config::load(matches));

    // Rewrite server server.properties file
    rewrite_server_properties(&config);

    // Start server service
    // TODO: start tokio runtime here?
    service::server::service(config).await
}

/// Rewrite server server.properties file with correct internal IP and port.
fn rewrite_server_properties(config: &Config) {
    // Rewrite must be enabled
    if !config.advanced.rewrite_server_properties {
        return;
    }

    // Ensure server directory is set, it must exist
    let dir = match &config.server.directory {
        Some(dir) => dir,
        None => {
            warn!(target: "lazymc", "Not rewriting {} file, server directory not configured (server.directory)", server_properties::FILE);
            return;
        }
    };

    // Build list of changes
    let changes = HashMap::from([
        ("server-ip", config.server.address.ip().to_string()),
        ("server-port", config.server.address.port().to_string()),
        ("query.port", config.server.address.port().to_string()),
    ]);

    // Rewrite file
    server_properties::rewrite_dir(dir, changes)
}
