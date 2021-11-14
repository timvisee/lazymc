use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

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
use crate::lobby;
use crate::proto::{self, Client, ClientInfo, ClientState, RawPacket};
use crate::server::{self, Server, State};
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

    // Remember inbound packets, used for client holding and forwarding
    let remember_inbound = config.join.methods.contains(&Method::Hold)
        || config.join.methods.contains(&Method::Forward);
    let mut inbound_history = BytesMut::new();

    let mut client_info = ClientInfo::empty();

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
            // Parse handshake
            let handshake = match Handshake::decode(&mut packet.data.as_slice()) {
                Ok(handshake) => handshake,
                Err(_) => {
                    debug!(target: "lazymc", "Got malformed handshake from client, disconnecting");
                    break;
                }
            };

            // Parse new state
            let new_state = match ClientState::from_id(handshake.next_state) {
                Some(state) => state,
                None => {
                    error!(target: "lazymc", "Client tried to switch into unknown protcol state ({}), disconnecting", handshake.next_state);
                    break;
                }
            };

            // Update client info and client state
            client_info
                .protocol_version
                .replace(handshake.protocol_version);
            client.set_state(new_state);

            // If login handshake and holding is enabled, hold packets
            if new_state == ClientState::Login && remember_inbound {
                inbound_history.extend(raw);
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
            // Try to get login username, update client info
            // TODO: we should always parse this packet successfully
            let username = LoginStart::decode(&mut packet.data.as_slice())
                .ok()
                .map(|p| p.name);
            client_info.username = username.clone();

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

            if !lobby::DONT_START_SERVER {
                // Start server if not starting yet
                Server::start(config.clone(), server.clone(), username).await;
            }

            // Lobby mode
            if lobby::USE_LOBBY {
                // // Hold login packet and remaining read bytes
                // hold_queue.extend(raw);
                // hold_queue.extend(buf.split_off(0));

                // Build queue with login packet and any additionally received
                let mut queue = BytesMut::with_capacity(raw.len() + buf.len());
                queue.extend(raw);
                queue.extend(buf.split_off(0));

                // Start lobby
                lobby::serve(client, client_info, inbound, config, server, queue).await?;
                return Ok(());
            }

            // Use join occupy methods
            for method in &config.join.methods {
                match method {
                    // Kick method, immediately kick client
                    Method::Kick => {
                        trace!(target: "lazymc", "Using kick method to occupy joining client");

                        // Select message and kick
                        let msg = match server.state() {
                            server::State::Starting
                            | server::State::Stopped
                            | server::State::Started => &config.join.kick.starting,
                            server::State::Stopping => &config.join.kick.stopping,
                        };
                        kick(msg, &mut writer).await?;
                        break;
                    }

                    // Hold method, hold client connection while server starts
                    Method::Hold => {
                        trace!(target: "lazymc", "Using hold method to occupy joining client");

                        // Server must be starting
                        if server.state() != State::Starting {
                            continue;
                        }

                        // Hold login packet and remaining read bytes
                        inbound_history.extend(&raw);
                        inbound_history.extend(buf.split_off(0));

                        // Start holding
                        if hold(&config, &server).await? {
                            service::server::route_proxy_queue(inbound, config, inbound_history);
                            return Ok(());
                        }
                    }

                    // Forward method, forward client connection while server starts
                    Method::Forward => {
                        trace!(target: "lazymc", "Using forward method to occupy joining client");

                        // Hold login packet and remaining read bytes
                        inbound_history.extend(&raw);
                        inbound_history.extend(buf.split_off(0));

                        // Forward client
                        debug!(target: "lazymc", "Forwarding client to {:?}!", config.join.forward.address);

                        service::server::route_proxy_address_queue(
                            inbound,
                            config.join.forward.address,
                            inbound_history,
                        );
                        return Ok(());

                        // TODO: do not consume client here, allow other join method on fail
                    }
                }
            }

            debug!(target: "lazymc", "No method left to occupy joining client, disconnecting");

            // Done occupying client, just disconnect
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
/// Returns holding status. `true` if client is held and it should be proxied, `false` it was held
/// but it timed out.
#[must_use]
pub async fn hold<'a>(config: &Config, server: &Server) -> Result<bool, ()> {
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
            return Ok(true);
        }

        // Server stopping/stopped, this shouldn't happen, kick
        Ok(false) => {
            warn!(target: "lazymc", "Server stopping for held client");
            return Ok(false);
        }

        // Timeout reached, kick with starting message
        Err(_) => {
            warn!(target: "lazymc", "Held client reached timeout of {}s", config.join.hold.timeout);
            return Ok(false);
        }
    }
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

    let response = RawPacket::new(proto::LOGIN_PACKET_ID_DISCONNECT, data).encode()?;
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
        if config.motd.from_server && status.is_some() {
            status.as_ref().unwrap().description.clone()
        } else {
            Message::new(Payload::text(match server.state() {
                server::State::Stopped | server::State::Started => &config.motd.sleeping,
                server::State::Starting => &config.motd.starting,
                server::State::Stopping => &config.motd.stopping,
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
