use std::sync::Arc;

use crate::config::Config;
use crate::probe;
use crate::server::Server;

/// Probe server.
pub async fn service(config: Arc<Config>, state: Arc<Server>) {
    // Only probe if Forge is enabled
    // TODO: do more comprehensive check for probe, only with forge and lobby?
    // TODO: add config option to probe on start
    if !config.server.forge {
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
