use tokio::net::TcpStream;

use crate::config::*;
use crate::net;
use crate::proto::action;
use crate::proto::client::Client;
use crate::server::{self, Server};

use super::MethodResult;

/// Kick the client.
pub async fn occupy(
    client: &Client,
    config: &Config,
    server: &Server,
    mut inbound: TcpStream,
) -> Result<MethodResult, ()> {
    trace!(target: "lazymc", "Using kick method to occupy joining client");

    // Select message and kick
    let msg = match server.state() {
        server::State::Starting | server::State::Stopped | server::State::Started => {
            &config.join.kick.starting
        }
        server::State::Stopping => &config.join.kick.stopping,
    };
    action::kick(client, msg, &mut inbound.split().1).await?;

    // Gracefully close connection
    net::close_tcp_stream(inbound).await.map_err(|_| ())?;

    Ok(MethodResult::Consumed)
}
