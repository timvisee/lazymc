use std::sync::Arc;

use crate::config::{Config, Method};
use crate::probe;
use crate::server::Server;

/// Probe server.
pub async fn service(config: Arc<Config>, state: Arc<Server>) {
    // Only probe if enabled or if we must
    if !config.server.probe_on_start && !must_probe(&config) {
        return;
    }

    // Probe
    match probe::probe(config, state).await {
        Ok(_) => info!(target: "lazymc::probe", "Succesfully probed server"),
        Err(_) => {
            error!(target: "lazymc::probe", "Failed to probe server, this may limit lazymc features")
        }
    }
}

/// Check whether we must probe.
fn must_probe(config: &Config) -> bool {
    // Must probe with lobby and Forge
    if config.server.forge && config.join.methods.contains(&Method::Lobby) {
        warn!(target: "lazymc::probe", "Starting server to probe for Forge lobby...");
        warn!(target: "lazymc::probe", "Set 'server.probe_on_start = true' to remove this warning");
        return true;
    }

    false
}
