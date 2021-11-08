use std::sync::Arc;

use futures::FutureExt;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::proto::Client;
use crate::proxy;
use crate::server::ServerState;
use crate::service;
use crate::status;
use crate::util::error::{quit_error, ErrorHints};

/// Start lazymc.
pub async fn service(config: Arc<Config>) -> Result<(), ()> {
    // Load server state
    let server_state = Arc::new(ServerState::default());

    // Listen for new connections
    // TODO: do not drop error here
    let listener = TcpListener::bind(config.public.address)
        .await
        .map_err(|err| {
            quit_error(
                anyhow!(err).context("Failed to start proxy server"),
                ErrorHints::default(),
            );
        })?;

    info!(
        "Proxying egress {} to ingress {}",
        config.public.address, config.server.address,
    );

    // Spawn server monitor and signal handler services
    tokio::spawn(service::monitor::service(
        config.clone(),
        server_state.clone(),
    ));
    tokio::spawn(service::signal::service(server_state.clone()));

    // Proxy all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        let client = Client::default();

        if !server_state.online() {
            // When server is not online, spawn a status server
            let transfer = status::serve(client, inbound, config.clone(), server_state.clone())
                .map(|r| {
                    if let Err(err) = r {
                        warn!("Failed to serve status: {:?}", err);
                    }
                });

            tokio::spawn(transfer);
        } else {
            // When server is online, proxy all
            let transfer = proxy::proxy(inbound, config.server.address).map(|r| {
                if let Err(err) = r {
                    warn!("Failed to proxy: {}", err);
                }
            });

            tokio::spawn(transfer);
        }
    }

    Ok(())
}
