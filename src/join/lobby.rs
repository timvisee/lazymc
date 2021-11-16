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

    // Start lobby
    lobby::serve(client, client_info, inbound, config, server, inbound_queue).await?;

    // TODO: do not consume client here, allow other join method on fail

    Ok(MethodResult::Consumed)
}
