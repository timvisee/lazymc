// TODO: remove all unwraps/expects here!

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::status::{PingRequest, PingResponse};
use rand::Rng;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::protocol::{self, ClientState, RawPacket};
use crate::server::ServerState;

/// Minecraft protocol version used when polling server status.
const PROTOCOL_VERSION: i32 = 754;

/// Monitor ping inverval in seconds.
const MONITOR_PING_INTERVAL: u64 = 2;

/// Ping timeout in seconds.
const PING_TIMEOUT: u64 = 8;

/// Poll server state.
///
/// Returns `true` if a ping succeeded.
pub async fn poll_server(addr: SocketAddr) -> bool {
    attempt_connect(addr).await.is_ok()
}

/// Monitor server.
pub async fn monitor_server(addr: SocketAddr, state: Arc<ServerState>) {
    loop {
        trace!("Polling {} ... ", addr);
        let online = poll_server(addr).await;

        state.set_online(online);

        tokio::time::sleep(Duration::from_secs(MONITOR_PING_INTERVAL)).await;
    }
}

/// Attemp to connect to the given server.
async fn attempt_connect(addr: SocketAddr) -> Result<(), ()> {
    let mut stream = TcpStream::connect(addr).await.map_err(|_| ())?;

    // Send handshake
    send_handshake(&mut stream, addr).await?;

    // Send ping request
    let token = send_ping(&mut stream).await?;

    // Wait for ping with timeout
    wait_for_ping_timeout(&mut stream, token).await?;

    Ok(())
}

/// Send handshake.
async fn send_handshake(stream: &mut TcpStream, addr: SocketAddr) -> Result<(), ()> {
    let handshake = Handshake {
        protocol_version: PROTOCOL_VERSION,
        server_addr: addr.ip().to_string(),
        server_port: addr.port(),
        next_state: ClientState::Status.to_id(),
    };

    let mut packet = Vec::new();
    handshake.encode(&mut packet).map_err(|_| ())?;

    let raw = RawPacket::new(protocol::HANDSHAKE_PACKET_ID_HANDSHAKE, packet)
        .encode()
        .map_err(|_| ())?;

    stream.write_all(&raw).await.map_err(|_| ())?;

    Ok(())
}

/// Send ping requets.
///
/// Returns sent ping time token on success.
async fn send_ping(stream: &mut TcpStream) -> Result<u64, ()> {
    // Generate a random ping token
    let token = rand::thread_rng().gen();

    let ping = PingRequest { time: token };

    let mut packet = Vec::new();
    ping.encode(&mut packet).map_err(|_| ())?;

    let raw = RawPacket::new(protocol::STATUS_PACKET_ID_PING, packet)
        .encode()
        .map_err(|_| ())?;

    stream.write_all(&raw).await.map_err(|_| ())?;

    Ok(token)
}

/// Wait for a ping response.
async fn wait_for_ping(stream: &mut TcpStream, token: u64) -> Result<(), ()> {
    // Get stream reader, set up buffer
    let (mut reader, mut _writer) = stream.split();
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, _raw) = match crate::read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => continue,
        };

        // Catch ping response
        if packet.id == protocol::STATUS_PACKET_ID_PING {
            let ping = PingResponse::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            // Ensure ping token is correct
            if ping.time != token {
                break;
            }

            return Ok(());
        }
    }

    // Some error occurred
    Err(())
}

/// Wait for a ping response with timeout.
async fn wait_for_ping_timeout(stream: &mut TcpStream, token: u64) -> Result<(), ()> {
    let ping = wait_for_ping(stream, token);
    tokio::time::timeout(Duration::from_secs(PING_TIMEOUT), ping)
        .await
        .map_err(|_| ())?
}
