use std::sync::Arc;

use bytes::BytesMut;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::data::server_status::*;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::{LoginDisconnect, LoginStart};
use minecraft_protocol::version::v1_14_4::status::StatusResponse;
use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::config::*;
use crate::proto::{self, Client, ClientState, RawPacket};
use crate::server::{self, Server};

/// Proxy the given inbound stream to a target address.
// TODO: do not drop error here, return Box<dyn Error>
pub async fn serve(
    client: Client,
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // Incoming buffer
    let mut buf = BytesMut::new();

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

        // Hijack login start
        if client.state() == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_START {
            // Try to get login username
            let username = LoginStart::decode(&mut packet.data.as_slice())
                .ok()
                .map(|p| p.name);

            // Select message
            let msg = match server.state() {
                server::State::Starting | server::State::Stopped | server::State::Started => {
                    &config.messages.login_starting
                }
                server::State::Stopping => &config.messages.login_stopping,
            };

            let packet = LoginDisconnect {
                reason: Message::new(Payload::text(msg)),
            };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(0, data).encode()?;
            writer.write_all(&response).await.map_err(|_| ())?;

            // Start server if not starting yet
            Server::start(config, server, username);
            break;
        }

        // Hijack handshake
        if client.state() == ClientState::Handshake && packet.id == proto::STATUS_PACKET_ID_STATUS {
            match Handshake::decode(&mut packet.data.as_slice()) {
                Ok(handshake) => {
                    // TODO: do not panic here
                    client.set_state(
                        ClientState::from_id(handshake.next_state)
                            .expect("unknown next client state"),
                    );
                }
                Err(_) => break,
            }
        }

        // Hijack server status packet
        if client.state() == ClientState::Status && packet.id == proto::STATUS_PACKET_ID_STATUS {
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
        if client.state() == ClientState::Status && packet.id == proto::STATUS_PACKET_ID_PING {
            writer.write_all(&raw).await.map_err(|_| ())?;
            continue;
        }

        // Show unhandled packet warning
        debug!(target: "lazymc", "Received unhandled packet:");
        debug!(target: "lazymc", "- State: {:?}", client.state());
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
