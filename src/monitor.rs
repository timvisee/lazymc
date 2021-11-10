// TODO: remove all unwraps/expects here!

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use minecraft_protocol::data::server_status::ServerStatus;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::status::{PingRequest, PingResponse, StatusResponse};
use rand::Rng;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time;

use crate::config::Config;
use crate::proto::{self, ClientState, RawPacket};
use crate::server::{Server, State};

/// Monitor ping inverval in seconds.
const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Status request timeout in seconds.
const STATUS_TIMEOUT: u64 = 8;

/// Ping request timeout in seconds.
const PING_TIMEOUT: u64 = 10;

/// Monitor server.
pub async fn monitor_server(config: Arc<Config>, server: Arc<Server>) {
    // Server address
    let addr = config.server.address;

    let mut poll_interval = time::interval(MONITOR_POLL_INTERVAL);

    loop {
        // Poll server state and update internal status
        trace!(target: "lazymc::monitor", "Fetching status for {} ... ", addr);
        let status = poll_server(&config, &server, addr).await;

        match status {
            // Got status, update
            Ok(Some(status)) => server.update_status(&config, Some(status)),

            // Error, reset status
            Err(_) => server.update_status(&config, None),

            // Didn't get status, but ping fallback worked, leave as-is, show warning
            Ok(None) => {
                warn!(target: "lazymc::monitor", "Failed to poll server status, ping fallback succeeded");
            }
        }

        // Sleep server when it's bedtime
        if server.should_sleep(&config) {
            info!(target: "lazymc::montior", "Server has been idle, sleeping...");
            if !server.stop(&config).await {
                warn!(target: "lazymc", "Failed to stop server");
            }
        }

        poll_interval.tick().await;
    }
}

/// Poll server state.
///
/// Returns `Ok` if status/ping succeeded, includes server status most of the time.
/// Returns `Err` if no connection could be established or if an error occurred.
pub async fn poll_server(
    config: &Config,
    server: &Server,
    addr: SocketAddr,
) -> Result<Option<ServerStatus>, ()> {
    // Fetch status
    if let Ok(status) = fetch_status(config, addr).await {
        return Ok(Some(status));
    }

    // Try ping fallback if server is currently started
    if server.state() == State::Started {
        do_ping(config, addr).await?;
    }

    Err(())
}

/// Attemp to fetch status from server.
async fn fetch_status(config: &Config, addr: SocketAddr) -> Result<ServerStatus, ()> {
    let mut stream = TcpStream::connect(addr).await.map_err(|_| ())?;

    send_handshake(&mut stream, &config, addr).await?;
    request_status(&mut stream).await?;
    wait_for_status_timeout(&mut stream).await
}

/// Attemp to ping server.
async fn do_ping(config: &Config, addr: SocketAddr) -> Result<(), ()> {
    let mut stream = TcpStream::connect(addr).await.map_err(|_| ())?;

    send_handshake(&mut stream, &config, addr).await?;
    let token = send_ping(&mut stream).await?;
    wait_for_ping_timeout(&mut stream, token).await
}

/// Send handshake.
async fn send_handshake(
    stream: &mut TcpStream,
    config: &Config,
    addr: SocketAddr,
) -> Result<(), ()> {
    let handshake = Handshake {
        protocol_version: config.public.protocol as i32,
        server_addr: addr.ip().to_string(),
        server_port: addr.port(),
        next_state: ClientState::Status.to_id(),
    };

    let mut packet = Vec::new();
    handshake.encode(&mut packet).map_err(|_| ())?;

    let raw = RawPacket::new(proto::HANDSHAKE_PACKET_ID_HANDSHAKE, packet)
        .encode()
        .map_err(|_| ())?;
    stream.write_all(&raw).await.map_err(|_| ())?;

    Ok(())
}

/// Send status request.
async fn request_status(stream: &mut TcpStream) -> Result<(), ()> {
    let raw = RawPacket::new(proto::STATUS_PACKET_ID_STATUS, vec![])
        .encode()
        .map_err(|_| ())?;
    stream.write_all(&raw).await.map_err(|_| ())?;
    Ok(())
}

/// Send status request.
async fn send_ping(stream: &mut TcpStream) -> Result<u64, ()> {
    let token = rand::thread_rng().gen();
    let ping = PingRequest { time: token };

    let mut packet = Vec::new();
    ping.encode(&mut packet).map_err(|_| ())?;

    let raw = RawPacket::new(proto::STATUS_PACKET_ID_PING, packet)
        .encode()
        .map_err(|_| ())?;
    stream.write_all(&raw).await.map_err(|_| ())?;
    Ok(token)
}

/// Wait for a status response.
async fn wait_for_status(stream: &mut TcpStream) -> Result<ServerStatus, ()> {
    // Get stream reader, set up buffer
    let (mut reader, mut _writer) = stream.split();
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, _raw) = match proto::read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => continue,
        };

        // Catch status response
        if packet.id == proto::STATUS_PACKET_ID_STATUS {
            let status = StatusResponse::decode(&mut packet.data.as_slice()).map_err(|_| ())?;
            return Ok(status.server_status);
        }
    }

    // Some error occurred
    Err(())
}

/// Wait for a status response.
async fn wait_for_status_timeout(stream: &mut TcpStream) -> Result<ServerStatus, ()> {
    let status = wait_for_status(stream);
    tokio::time::timeout(Duration::from_secs(STATUS_TIMEOUT), status)
        .await
        .map_err(|_| ())?
}

/// Wait for a status response.
async fn wait_for_ping(stream: &mut TcpStream, token: u64) -> Result<(), ()> {
    // Get stream reader, set up buffer
    let (mut reader, mut _writer) = stream.split();
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, _raw) = match proto::read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => continue,
        };

        // Catch ping response
        if packet.id == proto::STATUS_PACKET_ID_PING {
            let ping = PingResponse::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            // Ping token must match
            if ping.time == token {
                return Ok(());
            } else {
                debug!(target: "lazymc", "Got unmatched ping response when polling server status by ping");
            }
        }
    }

    // Some error occurred
    Err(())
}

/// Wait for a status response.
async fn wait_for_ping_timeout(stream: &mut TcpStream, token: u64) -> Result<(), ()> {
    let status = wait_for_ping(stream, token);
    tokio::time::timeout(Duration::from_secs(PING_TIMEOUT), status)
        .await
        .map_err(|_| ())?
}
