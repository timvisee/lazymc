use std::sync::Arc;

use bytes::BytesMut;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::data::server_status::*;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::login::LoginStart;
use minecraft_protocol::version::v1_14_4::status::StatusResponse;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::config::{Config, Server as ConfigServer};
use crate::join;
use crate::mc::favicon;
use crate::proto::action;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::packet::{self, RawPacket};
use crate::proto::packets;
use crate::server::{self, Server};

/// The ban message prefix.
const BAN_MESSAGE_PREFIX: &str = "Your IP address is banned from this server.\nReason: ";

/// Default ban reason if unknown.
const DEFAULT_BAN_REASON: &str = "Banned by an operator.";

/// The not-whitelisted kick message.
const WHITELIST_MESSAGE: &str = "You are not white-listed on this server!";

/// Server icon file path.
const SERVER_ICON_FILE: &str = "server-icon.png";

/// Proxy the given inbound stream to a target address.
// TODO: do not drop error here, return Box<dyn Error>
pub async fn serve(
    client: Client,
    mut inbound: TcpStream,
    server: Arc<Server>,
    mut inbound_history: BytesMut,
    mut client_info: ClientInfo,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // Incoming buffer and packet holding queue
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, raw) = match packet::read_packet(&client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc", "Closing connection, error occurred");
                break;
            }
        };

        // Grab client state
        let client_state = client.state();

        // Hijack ping packet
        if client_state == ClientState::Status && packet.id == packets::status::SERVER_PING {
            writer.write_all(&raw).await.map_err(|_| ())?;
            continue;
        }

        // Hijack server status packet
        if client_state == ClientState::Status && packet.id == packets::status::SERVER_STATUS {
            let server_status = server_status(&client_info, &server).await;
            let packet = StatusResponse { server_status };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(0, data).encode_with_len(&client)?;
            writer.write_all(&response).await.map_err(|_| ())?;

            continue;
        }

        // Hijack login start
        if client_state == ClientState::Login && packet.id == packets::login::SERVER_LOGIN_START {
            // Try to get login username, update client info
            // TODO: we should always parse this packet successfully
            let username = LoginStart::decode(&mut packet.data.as_slice())
                .ok()
                .map(|p| p.name);
            client_info.username = username.clone();

            // Kick if lockout is enabled
            if server.config.lockout.enabled {
                match username {
                    Some(username) => {
                        info!(target: "lazymc", "Kicked '{}' because lockout is enabled", username)
                    }
                    None => {
                        info!(target: "lazymc", "Kicked player because lockout is enabled")
                    }
                }
                action::kick(&client, &server.config.lockout.message, &mut writer).await?;
                break;
            }

            // Kick if client is banned
            if let Some(ban) = server.ban_entry(&client.peer.ip()).await {
                if ban.is_banned() {
                    let msg = if let Some(reason) = ban.reason {
                        info!(target: "lazymc", "Login from banned IP {} ({}), disconnecting", client.peer.ip(), &reason);
                        reason.to_string()
                    } else {
                        info!(target: "lazymc", "Login from banned IP {}, disconnecting", client.peer.ip());
                        DEFAULT_BAN_REASON.to_string()
                    };
                    action::kick(&client, &format!("{BAN_MESSAGE_PREFIX}{msg}"), &mut writer)
                        .await?;
                    break;
                }
            }

            // Kick if client is not whitelisted to wake server
            if let Some(ref username) = username {
                if !server.is_whitelisted(username).await {
                    info!(target: "lazymc", "User '{}' tried to wake server but is not whitelisted, disconnecting", username);
                    action::kick(&client, WHITELIST_MESSAGE, &mut writer).await?;
                    break;
                }
            }

            // Start server if not starting yet
            Server::start(server.clone(), username).await;

            // Remember inbound packets
            inbound_history.extend(&raw);
            inbound_history.extend(&buf);

            // Build inbound packet queue with everything from login start (including this)
            let mut login_queue = BytesMut::with_capacity(raw.len() + buf.len());
            login_queue.extend(&raw);
            login_queue.extend(&buf);

            // Buf is fully consumed here
            buf.clear();

            // Start occupying client
            join::occupy(
                client,
                client_info,
                server.clone(),
                inbound,
                inbound_history,
                login_queue,
            )
            .await?;
            return Ok(());
        }

        // Show unhandled packet warning
        debug!(target: "lazymc", "Got unhandled packet:");
        debug!(target: "lazymc", "- State: {:?}", client_state);
        debug!(target: "lazymc", "- Packet ID: {}", packet.id);
    }

    Ok(())
}

/// Build server status object to respond to client with.
async fn server_status(client_info: &ClientInfo, server: &Server) -> ServerStatus {
    let status = server.status().await;
    let server_state = server.state();

    // Respond with real server status if started
    if server_state == server::State::Started && status.is_some() {
        return status.as_ref().unwrap().clone();
    }

    // Select version and player max from last known server status
    let (version, max) = match status.as_ref() {
        Some(status) => (status.version.clone(), status.players.max),
        None => (
            ServerVersion {
                name: server.config.public.version.clone(),
                protocol: server.config.public.protocol,
            },
            0,
        ),
    };

    // Select description, use server MOTD if enabled, or use configured
    let description = {
        if server.config.motd.from_server && status.is_some() {
            status.as_ref().unwrap().description.clone()
        } else {
            Message::new(Payload::text(match server_state {
                server::State::Stopped | server::State::Started => &server.config.motd.sleeping,
                server::State::Starting => &server.config.motd.starting,
                server::State::Stopping => &server.config.motd.stopping,
            }))
        }
    };

    // Extract favicon from real server status, load from disk, or use default
    let mut favicon = None;
    if favicon::supports_favicon(client_info) {
        if server.config.motd.from_server && status.is_some() {
            favicon = status.as_ref().unwrap().favicon.clone()
        }
        if favicon.is_none() {
            favicon = Some(server_favicon(&server.config).await);
        }
    }

    // Build status resposne
    ServerStatus {
        version,
        description,
        players: OnlinePlayers {
            online: 0,
            max,
            sample: vec![],
        },
        favicon,
    }
}

/// Get server status favicon.
///
/// This always returns a favicon, returning the default one if none is set.
async fn server_favicon(config: &Config) -> String {
    // Get server dir
    let dir = match ConfigServer::server_directory(config) {
        Some(dir) => dir,
        None => return favicon::default_favicon(),
    };

    // Server icon file, ensure it exists
    let path = dir.join(SERVER_ICON_FILE);
    if !path.is_file() {
        return favicon::default_favicon();
    }

    // Read icon data
    let data = match fs::read(path).await.map_err(|err| {
        error!(target: "lazymc", "Failed to read favicon from {}: {}", SERVER_ICON_FILE, err);
    }) {
        Ok(data) => data,
        Err(err) => {
            error!(target: "lazymc::status", "Failed to load server icon from disk, using default: {:?}", err);
            return favicon::default_favicon();
        }
    };

    favicon::encode_favicon(&data)
}
