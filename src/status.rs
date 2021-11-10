use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::proxy;
use crate::server::State;
use bytes::BytesMut;
use futures::TryFutureExt;
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

/// Client holding server state poll interval.
const HOLD_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
                Ok(handshake) => {
                    // TODO: do not panic here
                    ClientState::from_id(handshake.next_state).expect("unknown next client state")
                }
                Err(_) => break,
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
            // Select version and player max from last known server status
            let (version, max) = match server.clone_status() {
                Some(status) => (status.version, status.players.max),
                None => (
                    ServerVersion {
                        name: config.public.version.clone(),
                        protocol: config.public.protocol,
                    },
                    0,
                ),
            };

            // Select description
            let description = match server.state() {
                server::State::Stopped | server::State::Started => &config.messages.motd_sleeping,
                server::State::Starting => &config.messages.motd_starting,
                server::State::Stopping => &config.messages.motd_stopping,
            };

            // Build status resposne
            let server_status = ServerStatus {
                version,
                description: Message::new(Payload::text(description)),
                players: OnlinePlayers {
                    online: 0,
                    max,
                    sample: vec![],
                },
            };
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

            // Start server if not starting yet
            Server::start(config.clone(), server.clone(), username);

            // Hold client if enabled and starting
            if config.time.hold() && server.state() == State::Starting {
                // Hold login packet and remaining read bytes
                hold_queue.extend(raw);
                hold_queue.extend(buf.split_off(0));

                // Start holding
                hold(inbound, config, server, &mut hold_queue).await?;
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
    holding: &mut BytesMut,
) -> Result<(), ()> {
    trace!(target: "lazymc", "Started holding client");

    // Set up polling interval, get timeout
    let mut poll_interval = time::interval(HOLD_POLL_INTERVAL);
    let since = Instant::now();
    let timeout = config.time.hold_client_for as u64;

    loop {
        // TODO: do not poll, wait for started signal instead (with timeout)
        poll_interval.tick().await;

        trace!("Polling server state for holding client...");

        match server.state() {
            // Still waiting on server start
            State::Starting => {
                trace!(target: "lazymc", "Server not ready, holding client for longer");

                // If hold timeout is reached, kick client
                if since.elapsed().as_secs() >= timeout {
                    warn!(target: "lazymc", "Holding client reached timeout of {}s, disconnecting", timeout);
                    kick(&config.messages.login_starting, &mut inbound.split().1).await?;
                    return Ok(());
                }

                continue;
            }

            // Server started, start relaying and proxy
            State::Started => {
                // TODO: drop client if already disconnected

                // Relay client to proxy
                info!(target: "lazymc", "Server ready for held client, relaying to server");
                proxy::proxy_with_queue(inbound, config.server.address, &holding)
                    .map_err(|_| ())
                    .await?;
                return Ok(());
            }

            // Server stopping, this shouldn't happen, kick
            State::Stopping => {
                warn!(target: "lazymc", "Server stopping for held client, disconnecting");
                kick(&config.messages.login_stopping, &mut inbound.split().1).await?;
                break;
            }

            // Server stopped, this shouldn't happen, disconnect
            State::Stopped => {
                error!(target: "lazymc", "Server stopped for held client, disconnecting");
                break;
            }
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
async fn kick<'a>(msg: &str, writer: &mut WriteHalf<'a>) -> Result<(), ()> {
    let packet = LoginDisconnect {
        reason: Message::new(Payload::text(msg)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(0, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())
}
