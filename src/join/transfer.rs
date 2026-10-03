use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::{select, time};

use crate::config::*;
use crate::mc::uuid;
use crate::net;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::packet::{self, RawPacket};
use crate::server::{Server, State};
use crate::types;

use super::MethodResult;

/// Minimum protocol version that supports the transfer packet (1.20.5).
const MIN_PROTOCOL_TRANSFER: u32 = 766;

/// Clientbound login success packet ID, login state.
const PACKET_LOGIN_SUCCESS: u8 = 0x02;

/// Serverbound login acknowledged packet ID, login state.
const PACKET_LOGIN_ACKNOWLEDGED: u8 = 0x03;

/// Keep alive packet ID, configuration state, both directions.
const PACKET_CONFIG_KEEP_ALIVE: u8 = 0x04;

/// Clientbound transfer packet ID, configuration state.
const PACKET_CONFIG_TRANSFER: u8 = 0x0b;

/// Hold the client in the configuration state, then transfer it to the server.
///
/// Unlike the hold method, this is not bound by the 30 second client timeout,
/// because the client is kept alive with keep alive packets. Once the server is
/// ready the client is told to reconnect to it with a transfer packet, which
/// makes it go through a full handshake with the real server. This keeps modded
/// servers working, because no registry or mod data has to be emulated here.
pub async fn occupy(
    client: &Client,
    client_info: ClientInfo,
    config: Arc<Config>,
    server: Arc<Server>,
    mut inbound: TcpStream,
) -> Result<MethodResult, ()> {
    trace!(target: "lazymc", "Using transfer method to occupy joining client");

    // Server must be starting
    if server.state() != State::Starting {
        return Ok(MethodResult::Continue(inbound));
    }

    // Client must support the transfer packet
    let protocol = client_info.protocol().unwrap_or(0);
    if protocol < MIN_PROTOCOL_TRANSFER {
        warn!(
            target: "lazymc",
            "Client protocol {} is too old for transfer method, needs {} (1.20.5) or above",
            protocol, MIN_PROTOCOL_TRANSFER,
        );
        return Ok(MethodResult::Continue(inbound));
    }

    let username = match client_info.username.clone() {
        Some(username) => username,
        None => {
            error!(target: "lazymc", "No username for transfer method, disconnecting");
            return Ok(MethodResult::Continue(inbound));
        }
    };

    let (mut reader, mut writer) = inbound.split();

    // Send login success, moving the client towards the configuration state
    let payload = encode_login_success(&username)?;
    let packet = RawPacket::new(PACKET_LOGIN_SUCCESS, payload).encode_with_len(client)?;
    writer.write_all(&packet).await.map_err(|_| ())?;
    trace!(target: "lazymc", "Sent login success to client");

    // Await login acknowledged, client enters the configuration state
    let mut buf = BytesMut::new();
    loop {
        let (packet, _raw) = match packet::read_packet(client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => return Ok(MethodResult::Consumed),
            Err(_) => return Ok(MethodResult::Consumed),
        };

        if packet.id == PACKET_LOGIN_ACKNOWLEDGED {
            break;
        }
    }

    client.set_state(ClientState::Configuration);
    debug!(target: "lazymc", "Client entered configuration state, holding until server is ready");

    // Keep the client alive until the server is started
    let interval = Duration::from_secs(config.join.transfer.keep_alive_interval.max(1) as u64);
    let timeout = Duration::from_secs(config.join.transfer.timeout as u64);

    let task_keep_alive = async {
        let mut tick = time::interval(interval);
        let mut id: i64 = 0;
        loop {
            tick.tick().await;
            id += 1;

            let packet = match RawPacket::new(PACKET_CONFIG_KEEP_ALIVE, id.to_be_bytes().to_vec())
                .encode_with_len(client)
            {
                Ok(packet) => packet,
                Err(_) => return,
            };
            if writer.write_all(&packet).await.is_err() {
                trace!(target: "lazymc", "Client gone while keeping alive");
                return;
            }
            trace!(target: "lazymc", "Sent keep alive to held client");
        }
    };

    let task_wait = async {
        let mut state = server.state_receiver();
        loop {
            state.changed().await.unwrap();
            match state.borrow().deref() {
                State::Starting => continue,
                State::Started => break true,
                State::Stopping | State::Stopped => break false,
            }
        }
    };

    let ready = select! {
        ready = time::timeout(timeout, task_wait) => match ready {
            Ok(ready) => ready,
            Err(_) => {
                warn!(target: "lazymc", "Transferred client reached timeout of {}s", config.join.transfer.timeout);
                false
            }
        },
        _ = task_keep_alive => {
            trace!(target: "lazymc", "Keep alive loop ended, client probably disconnected");
            return Ok(MethodResult::Consumed);
        }
    };

    if !ready {
        return Ok(MethodResult::Consumed);
    }

    // Server is ready, transfer the client to it
    let (host, port) = transfer_target(&config);
    info!(target: "lazymc", "Server ready, transferring client to {}:{}", host, port);

    let mut payload = encode_string(&host)?;
    payload.append(&mut types::encode_var_int(port as i32)?);
    let packet = RawPacket::new(PACKET_CONFIG_TRANSFER, payload).encode_with_len(client)?;
    writer.write_all(&packet).await.map_err(|_| ())?;

    // Give the client a moment to act on it, then close
    time::sleep(Duration::from_millis(500)).await;
    let _ = net::close_tcp_stream(inbound).await;

    Ok(MethodResult::Consumed)
}

/// Address the client should be transferred to.
///
/// This must be reachable by the client, so it defaults to the public address
/// lazymc listens on rather than the internal server address.
fn transfer_target(config: &Config) -> (String, u16) {
    if let Some(address) = &config.join.transfer.address {
        if let Some((host, port)) = address.rsplit_once(':') {
            if let Ok(port) = port.parse::<u16>() {
                return (host.to_string(), port);
            }
        }
        return (address.clone(), config.public.address.port());
    }

    (
        config.public.address.ip().to_string(),
        config.public.address.port(),
    )
}

/// Encode a login success packet body for 1.20.5 and above.
///
/// Layout: UUID, username, property count, strict error handling flag.
fn encode_login_success(username: &str) -> Result<Vec<u8>, ()> {
    let mut data = uuid::offline_player_uuid(username).as_bytes().to_vec();
    data.append(&mut encode_string(username)?);
    data.append(&mut types::encode_var_int(0)?);
    data.push(0x00);
    Ok(data)
}

/// Encode a length prefixed UTF-8 string.
fn encode_string(value: &str) -> Result<Vec<u8>, ()> {
    let bytes = value.as_bytes();
    let mut data = types::encode_var_int(bytes.len() as i32)?;
    data.extend_from_slice(bytes);
    Ok(data)
}
