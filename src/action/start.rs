use std::sync::Arc;

use clap::ArgMatches;

use crate::config;
use crate::service;

/// Start lazymc.
pub async fn invoke(matches: &ArgMatches) -> Result<(), ()> {
    // Load config
    let config = Arc::new(config::load(matches));

    // Start server service
    // TODO: start tokio runtime here?
    service::server::service(config).await
}
