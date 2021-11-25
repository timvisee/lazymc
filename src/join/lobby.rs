use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::TcpStream;

use crate::config::*;
use crate::lobby;
use crate::proto::client::{Client, ClientInfo};
use crate::server::Server;

use super::MethodResult;

/// Lobby the client.
pub async fn occupy(
    client: &Client,
    client_info: ClientInfo,
    config: Arc<Config>,
    server: Arc<Server>,
    inbound: TcpStream,
    inbound_queue: BytesMut,
) -> Result<MethodResult, ()> {
    trace!(target: "lazymc", "Using lobby method to occupy joining client");

    // Must be ready to lobby
    if must_still_probe(&config, &server).await {
        warn!(target: "lazymc", "Client connected but lobby is not ready, using next join method, probing not completed");
        return Ok(MethodResult::Continue(inbound));
    }

    // Start lobby
    lobby::serve(client, client_info, inbound, config, server, inbound_queue).await?;

    // TODO: do not consume client here, allow other join method on fail

    Ok(MethodResult::Consumed)
}

/// Check whether we still have to probe before we can use the lobby.
async fn must_still_probe(config: &Config, server: &Server) -> bool {
    must_probe(config) && server.probed_join_game.read().await.is_none()
}

/// Check whether we must have probed data.
fn must_probe(config: &Config) -> bool {
    config.server.forge
}
