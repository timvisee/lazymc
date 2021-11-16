use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::TcpStream;

use crate::config::*;
use crate::net;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::server::Server;

pub mod forward;
pub mod hold;
pub mod kick;
#[cfg(feature = "lobby")]
pub mod lobby;

/// A result returned by a join occupy method.
pub enum MethodResult {
    /// Client is consumed.
    Consumed,

    /// Method is done, continue with the next.
    Continue(TcpStream),
}

/// Start occupying client.
///
/// This assumes the login start packet has just been received.
pub async fn occupy(
    client: Client,
    #[allow(unused_variables)] client_info: ClientInfo,
    config: Arc<Config>,
    server: Arc<Server>,
    mut inbound: TcpStream,
    mut inbound_history: BytesMut,
    #[allow(unused_variables)] login_queue: BytesMut,
) -> Result<(), ()> {
    // Assert state is correct
    assert_eq!(
        client.state(),
        ClientState::Login,
        "when occupying client, it should be in login state"
    );

    // Go through all configured join methods
    for method in &config.join.methods {
        // Invoke method, take result
        let result = match method {
            // Kick method, immediately kick client
            Method::Kick => kick::occupy(&client, &config, &server, inbound).await?,

            // Hold method, hold client connection while server starts
            Method::Hold => {
                hold::occupy(
                    config.clone(),
                    server.clone(),
                    inbound,
                    &mut inbound_history,
                )
                .await?
            }

            // Forward method, forward client connection while server starts
            Method::Forward => {
                forward::occupy(config.clone(), inbound, &mut inbound_history).await?
            }

            // Lobby method, keep client in lobby while server starts
            #[cfg(feature = "lobby")]
            Method::Lobby => {
                lobby::occupy(
                    &client,
                    client_info.clone(),
                    config.clone(),
                    server.clone(),
                    inbound,
                    login_queue.clone(),
                )
                .await?
            }

            // Lobby method, keep client in lobby while server starts
            #[cfg(not(feature = "lobby"))]
            Method::Lobby => {
                error!(target: "lazymc", "Lobby join method not supported in this lazymc build");
                MethodResult::Continue(inbound)
            }
        };

        // Handle method result
        match result {
            MethodResult::Consumed => return Ok(()),
            MethodResult::Continue(stream) => {
                inbound = stream;
                continue;
            }
        }
    }

    debug!(target: "lazymc", "No method left to occupy joining client, disconnecting");

    // Gracefully close connection
    net::close_tcp_stream(inbound).await.map_err(|_| ())?;

    Ok(())
}
