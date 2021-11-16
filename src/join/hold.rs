use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::TcpStream;
use tokio::time;

use crate::config::*;
use crate::server::{Server, State};
use crate::service;

use super::MethodResult;

/// Hold the client.
pub async fn occupy(
    config: Arc<Config>,
    server: Arc<Server>,
    inbound: TcpStream,
    inbound_history: &mut BytesMut,
) -> Result<MethodResult, ()> {
    trace!(target: "lazymc", "Using hold method to occupy joining client");

    // Server must be starting
    if server.state() != State::Starting {
        return Ok(MethodResult::Continue(inbound));
    }

    // Start holding, consume client
    if hold(&config, &server).await? {
        service::server::route_proxy_queue(inbound, config, inbound_history.clone());
        return Ok(MethodResult::Consumed);
    }

    Ok(MethodResult::Continue(inbound))
}

/// Hold a client while server starts.
///
/// Returns holding status. `true` if client is held and it should be proxied, `false` it was held
/// but it timed out.
async fn hold<'a>(config: &Config, server: &Server) -> Result<bool, ()> {
    trace!(target: "lazymc", "Started holding client");

    // A task to wait for suitable server state
    // Waits for started state, errors if stopping/stopped state is reached
    let task_wait = async {
        let mut state = server.state_receiver();
        loop {
            // Wait for state change
            state.changed().await.unwrap();

            match state.borrow().deref() {
                // Still waiting on server start
                State::Starting => {
                    trace!(target: "lazymc", "Server not ready, holding client for longer");
                    continue;
                }

                // Server started, start relaying and proxy
                State::Started => {
                    break true;
                }

                // Server stopping, this shouldn't happen, kick
                State::Stopping => {
                    warn!(target: "lazymc", "Server stopping for held client, disconnecting");
                    break false;
                }

                // Server stopped, this shouldn't happen, disconnect
                State::Stopped => {
                    error!(target: "lazymc", "Server stopped for held client, disconnecting");
                    break false;
                }
            }
        }
    };

    // Wait for server state with timeout
    let timeout = Duration::from_secs(config.join.hold.timeout as u64);
    match time::timeout(timeout, task_wait).await {
        // Relay client to proxy
        Ok(true) => {
            info!(target: "lazymc", "Server ready for held client, relaying to server");
            Ok(true)
        }

        // Server stopping/stopped, this shouldn't happen, kick
        Ok(false) => {
            warn!(target: "lazymc", "Server stopping for held client");
            Ok(false)
        }

        // Timeout reached, kick with starting message
        Err(_) => {
            warn!(target: "lazymc", "Held client reached timeout of {}s", config.join.hold.timeout);
            Ok(false)
        }
    }
}
