use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use crate::server::State;
use bytes::BytesMut;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::data::server_status::*;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::{LoginDisconnect, LoginStart};
use minecraft_protocol::version::v1_14_4::status::StatusResponse;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::tcp::WriteHalf;
use tokio::net::TcpStream;
use tokio::time;

use crate::config::*;
use crate::proto::{self, Client, ClientState, RawPacket};
use crate::server::{self, Server};
use crate::service;

/// Proxy the given inbound stream to a target address.
// TODO: do not drop error here, return Box<dyn Error>
pub async fn serve(
    client: Client,
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // Incoming buffer and packet holding queue
    let mut buf = BytesMut::new();
    let mut hold_queue = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, raw) = match proto::read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc", "Closing connection, error occurred");
                break;
            }
        };

        // Grab client state
        let client_state = client.state();

        // Hijack handshake
        if client_state == ClientState::Handshake && packet.id == proto::STATUS_PACKET_ID_STATUS {
            // Parse handshake, grab new state
            let new_state = match Handshake::decode(&mut packet.data.as_slice()) {
                Ok(handshake) => match ClientState::from_id(handshake.next_state) {
                    Some(state) => state,
                    None => {
                        error!(target: "lazymc", "Client tried to switch into unknown protcol state ({}), disconnecting", handshake.next_state);
                        break;
                    }
                },
                Err(_) => {
                    debug!(target: "lazymc", "Got malformed handshake from client, disconnecting");
                    break;
                }
            };

            // Update client state
            client.set_state(new_state);

            // If login handshake and holding is enabled, hold packets
            if new_state == ClientState::Login && config.time.hold() {
                hold_queue.extend(raw);
            }

            continue;
        }

        // Hijack server status packet
        if client_state == ClientState::Status && packet.id == proto::STATUS_PACKET_ID_STATUS {
            let server_status = server_status(&config, &server).await;
            let packet = StatusResponse { server_status };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(0, data).encode()?;
            writer.write_all(&response).await.map_err(|_| ())?;

            continue;
        }

        // Hijack ping packet
        if client_state == ClientState::Status && packet.id == proto::STATUS_PACKET_ID_PING {
            writer.write_all(&raw).await.map_err(|_| ())?;
            continue;
        }

        // Hijack login start
        if client_state == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_START {
            // Try to get login username
            let username = LoginStart::decode(&mut packet.data.as_slice())
                .ok()
                .map(|p| p.name);

            // Kick if lockout is enabled
            if config.lockout.enabled {
                match username {
                    Some(username) => {
                        info!(target: "lazymc", "Kicked '{}' because lockout is enabled", username)
                    }
                    None => info!(target: "lazymc", "Kicked player because lockout is enabled"),
                }
                kick(&config.lockout.message, &mut writer).await?;
                break;
            }

            // Start server if not starting yet
            Server::start(config.clone(), server.clone(), username).await;

            // Hold client if enabled and starting
            if config.time.hold() && server.state() == State::Starting {
                // Hold login packet and remaining read bytes
                hold_queue.extend(raw);
                hold_queue.extend(buf.split_off(0));

                // Start holding
                hold(inbound, config, server, hold_queue).await?;
                return Ok(());
            }

            // Select message and kick
            let msg = match server.state() {
                server::State::Starting | server::State::Stopped | server::State::Started => {
                    &config.messages.login_starting
                }
                server::State::Stopping => &config.messages.login_stopping,
            };
            kick(msg, &mut writer).await?;

            break;
        }

        // Show unhandled packet warning
        debug!(target: "lazymc", "Received unhandled packet:");
        debug!(target: "lazymc", "- State: {:?}", client_state);
        debug!(target: "lazymc", "- Packet ID: {}", packet.id);
    }

    // Gracefully close connection
    match writer.shutdown().await {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotConnected => {}
        Err(_) => return Err(()),
    }

    Ok(())
}

/// Hold a client while server starts.
///
/// Relays client to proxy once server is ready.
pub async fn hold<'a>(
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
    hold_queue: BytesMut,
) -> Result<(), ()> {
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
    let timeout = Duration::from_secs(config.time.hold_client_for as u64);
    match time::timeout(timeout, task_wait).await {
        // Relay client to proxy
        Ok(true) => {
            info!(target: "lazymc", "Server ready for held client, relaying to server");
            service::server::route_proxy_queue(inbound, config, hold_queue);
            return Ok(());
        }

        // Server stopping/stopped, this shouldn't happen, kick
        Ok(false) => {
            warn!(target: "lazymc", "Server stopping for held client, disconnecting");
            kick(&config.messages.login_stopping, &mut inbound.split().1).await?;
        }

        // Timeout reached, kick with starting message
        Err(_) => {
            warn!(target: "lazymc", "Held client reached timeout of {}s, disconnecting", config.time.hold_client_for);
            kick(&config.messages.login_starting, &mut inbound.split().1).await?;
        }
    }

    // Gracefully close connection
    match inbound.shutdown().await {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotConnected => {}
        Err(_) => return Err(()),
    }

    Ok(())
}

/// Kick client with a message.
///
/// Should close connection afterwards.
async fn kick(msg: &str, writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let packet = LoginDisconnect {
        reason: Message::new(Payload::text(msg)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(0, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())
}

/// Build server status object to respond to client with.
async fn server_status(config: &Config, server: &Server) -> ServerStatus {
    let status = server.status().await;

    // Select version and player max from last known server status
    let (version, max) = match status.as_ref() {
        Some(status) => (status.version.clone(), status.players.max),
        None => (
            ServerVersion {
                name: config.public.version.clone(),
                protocol: config.public.protocol,
            },
            0,
        ),
    };

    // Select description, use server MOTD if enabled, or use configured
    let description = {
        if config.messages.use_server_motd && status.is_some() {
            status.as_ref().unwrap().description.clone()
        } else {
            Message::new(Payload::text(match server.state() {
                server::State::Stopped | server::State::Started => &config.messages.motd_sleeping,
                server::State::Starting => &config.messages.motd_starting,
                server::State::Stopping => &config.messages.motd_stopping,
            }))
        }
    };

    // Build status resposne
    ServerStatus {
        version,
        description,
        players: OnlinePlayers {
            online: 0,
            max,
            sample: vec![],
        },
    }
}
