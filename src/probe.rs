use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::{
    LoginPluginRequest, LoginPluginResponse, LoginStart, SetCompression,
};
use tokio::net::TcpStream;
use tokio::time;

use crate::config::Config;
use crate::forge;
use crate::net;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::packets::play::join_game::JoinGameData;
use crate::proto::{self, packet, packets};
use crate::server::{Server, State};

/// Minecraft username to use for probing the server.
const PROBE_USER: &str = "_lazymc_probe";

/// Timeout for probe user connecting to the server.
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time the probe may wait for the server to come online.
const PROBE_ONLINE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Timeout for receiving join game packet.
///
/// When the play state is reached, the server should immeditely respond with a join game packet.
/// This defines the maximum timeout for waiting on it.
const PROBE_JOIN_GAME_TIMEOUT: Duration = Duration::from_secs(20);

/// Connect to the Minecraft server and probe useful details from it.
pub async fn probe(config: Arc<Config>, server: Arc<Server>) -> Result<(), ()> {
    debug!(target: "lazymc::probe", "Starting server probe...");

    // Start server if not starting already
    if Server::start(config.clone(), server.clone(), None).await {
        info!(target: "lazymc::probe", "Starting server to probe...");
    }

    // Wait for server to come online
    if !wait_until_online(&server).await? {
        warn!(target: "lazymc::probe", "Couldn't probe server, failed to wait for server to come online");
        return Err(());
    }

    debug!(target: "lazymc::probe", "Connecting to server to probe details...");

    // Connect to server, record Forge payload
    let forge_payload = connect_to_server(&config, &server).await?;
    *server.forge_payload.write().await = forge_payload;

    Ok(())
}

/// Wait for the server to come online.
///
/// Returns `true` when it is online.
async fn wait_until_online<'a>(server: &Server) -> Result<bool, ()> {
    trace!(target: "lazymc::probe", "Waiting for server to come online...");

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
                    continue;
                }

                // Server started, start relaying and proxy
                State::Started => {
                    break true;
                }

                // Server stopping, this shouldn't happen, skip
                State::Stopping => {
                    warn!(target: "lazymc::probe", "Server stopping while trying to probe, skipping");
                    break false;
                }

                // Server stopped, this shouldn't happen, skip
                State::Stopped => {
                    error!(target: "lazymc::probe", "Server stopped while trying to probe, skipping");
                    break false;
                }
            }
        }
    };

    // Wait for server state with timeout
    match time::timeout(PROBE_ONLINE_TIMEOUT, task_wait).await {
        Ok(online) => Ok(online),

        // Timeout reached, kick with starting message
        Err(_) => {
            warn!(target: "lazymc::probe", "Probe waited for server to come online but timed out after {}s", PROBE_ONLINE_TIMEOUT.as_secs());
            Ok(false)
        }
    }
}

/// Create connection to the server, with timeout.
///
/// This will initialize the connection to the play state. Client details are used.
///
/// Returns recorded Forge login payload if any.
async fn connect_to_server(config: &Config, server: &Server) -> Result<Vec<Vec<u8>>, ()> {
    time::timeout(
        PROBE_CONNECT_TIMEOUT,
        connect_to_server_no_timeout(config, server),
    )
    .await
    .map_err(|_| {
        error!(target: "lazymc::probe", "Probe tried to connect to server but timed out after {}s", PROBE_CONNECT_TIMEOUT.as_secs());
    })?
}

/// Create connection to the server, with no timeout.
///
/// This will initialize the connection to the play state. Client details are used.
///
/// Returns recorded Forge login payload if any.
// TODO: clean this up
async fn connect_to_server_no_timeout(
    config: &Config,
    server: &Server,
) -> Result<Vec<Vec<u8>>, ()> {
    // Open connection
    // TODO: on connect fail, ping server and redirect to serve_status if offline
    let mut outbound = TcpStream::connect(config.server.address)
        .await
        .map_err(|_| ())?;

    // Construct temporary server client
    let tmp_client = match outbound.local_addr() {
        Ok(addr) => Client::new(addr),
        Err(_) => Client::dummy(),
    };
    tmp_client.set_state(ClientState::Login);

    // Construct client info
    let mut tmp_client_info = ClientInfo::empty();
    tmp_client_info.protocol.replace(config.public.protocol);

    let (mut reader, mut writer) = outbound.split();

    // Select server address to use, add magic if Forge
    let server_addr = if config.server.forge {
        format!("{}{}", config.server.address.ip(), forge::STATUS_MAGIC)
    } else {
        config.server.address.ip().to_string()
    };

    // Send handshake packet
    packet::write_packet(
        Handshake {
            protocol_version: config.public.protocol as i32,
            server_addr,
            server_port: config.server.address.port(),
            next_state: ClientState::Login.to_id(),
        },
        &tmp_client,
        &mut writer,
    )
    .await?;

    // Request login start
    packet::write_packet(
        LoginStart {
            name: PROBE_USER.into(),
        },
        &tmp_client,
        &mut writer,
    )
    .await?;

    // Incoming buffer, record Forge plugin request payload
    let mut buf = BytesMut::new();
    let mut forge_payload = Vec::new();

    loop {
        // Read packet from stream
        let (packet, raw) = match packet::read_packet(&tmp_client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc::forge", "Closing connection, error occurred");
                break;
            }
        };

        // Grab client state
        let client_state = tmp_client.state();

        // Catch set compression
        if client_state == ClientState::Login && packet.id == packets::login::CLIENT_SET_COMPRESSION
        {
            // Decode compression packet
            let set_compression =
                SetCompression::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            // Client and server compression threshold should match, show warning if not
            if set_compression.threshold != proto::COMPRESSION_THRESHOLD {
                error!(
                    target: "lazymc::forge",
                    "Compression threshold sent to lobby client does not match threshold from server, this may cause errors (client: {}, server: {})",
                    proto::COMPRESSION_THRESHOLD,
                    set_compression.threshold
                );
            }

            // Set client compression
            tmp_client.set_compression(set_compression.threshold);
            continue;
        }

        // Catch login plugin request
        if client_state == ClientState::Login
            && packet.id == packets::login::CLIENT_LOGIN_PLUGIN_REQUEST
        {
            // Decode login plugin request packet
            let plugin_request = LoginPluginRequest::decode(&mut packet.data.as_slice()).map_err(|err| {
                error!(target: "lazymc::probe", "Failed to decode login plugin request from server, cannot respond properly: {:?}", err);
            })?;

            // Handle plugin requests for Forge
            if config.server.forge {
                // Record Forge login payload
                forge_payload.push(raw);

                // Respond to Forge login plugin request
                forge::respond_login_plugin_request(&tmp_client, plugin_request, &mut writer)
                    .await?;
                continue;
            }

            warn!(target: "lazymc::probe", "Got unexpected login plugin request, responding with error");

            // Respond with plugin response failure
            packet::write_packet(
                LoginPluginResponse {
                    message_id: plugin_request.message_id,
                    successful: false,
                    data: vec![],
                },
                &tmp_client,
                &mut writer,
            )
            .await?;

            continue;
        }

        // Hijack login success
        if client_state == ClientState::Login && packet.id == packets::login::CLIENT_LOGIN_SUCCESS {
            trace!(target: "lazymc::probe", "Got login success from server connection, change to play mode");

            // Switch to play state
            tmp_client.set_state(ClientState::Play);

            // Wait to catch join game packet
            let join_game_data =
                wait_for_server_join_game(&tmp_client, &tmp_client_info, &mut outbound, &mut buf)
                    .await?;
            server
                .probed_join_game
                .write()
                .await
                .replace(join_game_data);

            // Gracefully close connection
            let _ = net::close_tcp_stream(outbound).await;

            return Ok(forge_payload);
        }

        // Show unhandled packet warning
        debug!(target: "lazymc::forge", "Got unhandled packet from server in connect_to_server:");
        debug!(target: "lazymc::forge", "- State: {:?}", client_state);
        debug!(target: "lazymc::forge", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // Gracefully close connection
    net::close_tcp_stream(outbound).await.map_err(|_| ())?;

    Err(())
}

/// Wait for join game packet on server connection, with timeout.
///
/// This parses, consumes and returns the packet.
async fn wait_for_server_join_game(
    client: &Client,
    client_info: &ClientInfo,
    outbound: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<JoinGameData, ()> {
    time::timeout(
        PROBE_JOIN_GAME_TIMEOUT,
        wait_for_server_join_game_no_timeout(client, client_info, outbound, buf),
    )
    .await
    .map_err(|_| {
        error!(target: "lazymc::probe", "Waiting for for game data from server for probe client timed out after {}s", PROBE_JOIN_GAME_TIMEOUT.as_secs());
    })?
}

/// Wait for join game packet on server connection, with no timeout.
///
/// This parses, consumes and returns the packet.
// TODO: clean this up
// TODO: do not drop error here, return Box<dyn Error>
async fn wait_for_server_join_game_no_timeout(
    client: &Client,
    client_info: &ClientInfo,
    outbound: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<JoinGameData, ()> {
    let (mut reader, mut _writer) = outbound.split();

    loop {
        // Read packet from stream
        let (packet, _raw) = match packet::read_packet(client, buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc::probe", "Closing connection, error occurred");
                break;
            }
        };

        // Catch join game
        if packets::play::join_game::is_packet(client_info, packet.id) {
            // Parse join game data
            let join_game_data = JoinGameData::from_packet(client_info, packet).map_err(|err| {
                warn!(target: "lazymc::probe", "Failed to parse join game packet: {:?}", err);
            })?;

            return Ok(join_game_data);
        }

        // Show unhandled packet warning
        debug!(target: "lazymc::probe", "Got unhandled packet from server in wait_for_server_join_game:");
        debug!(target: "lazymc::probe", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // Gracefully close connection
    net::close_tcp_stream_ref(outbound).await.map_err(|_| ())?;

    Err(())
}
