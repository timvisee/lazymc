use std::io::ErrorKind;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use futures::FutureExt;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::version::v1_14_4::login::{
    LoginPluginRequest, LoginPluginResponse, LoginStart, LoginSuccess, SetCompression,
};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::time;

use crate::config::*;
use crate::forge;
use crate::mc::uuid;
use crate::net;
use crate::proto;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::packets::play::join_game::JoinGameData;
use crate::proto::{packet, packets};
use crate::proxy;
use crate::server::{Server, State};

/// Interval to send keep-alive packets at.
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Timeout for creating new server connection for lobby client.
const SERVER_CONNECT_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Timeout for server sending join game packet.
///
/// When the play state is reached, the server should immeditely respond with a join game packet.
/// This defines the maximum timeout for waiting on it.
const SERVER_JOIN_GAME_TIMEOUT: Duration = Duration::from_secs(20);

/// Time to wait before responding to newly connected server.
///
/// Notchian servers are slow, we must wait a little before sending play packets, because the
/// server needs time to transition the client into this state.
/// See warning at: <https://wiki.vg/Protocol#Login_Success>
const SERVER_WARMUP: Duration = Duration::from_secs(1);

/// Serve lobby service for given client connection.
///
/// The client must be in the login state, or this will error.
// TODO: do not drop error here, return Box<dyn Error>
// TODO: on error, nicely kick client with message
pub async fn serve(
    client: &Client,
    client_info: ClientInfo,
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
    queue: BytesMut,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // Client must be in login state
    if client.state() != ClientState::Login {
        error!(target: "lazymc::lobby", "Client reached lobby service with invalid state: {:?}", client.state());
        return Err(());
    }

    // We must have useful client info
    if client_info.username.is_none() {
        error!(target: "lazymc::lobby", "Client username is unknown, closing connection");
        return Err(());
    }

    // Incoming buffer
    let mut inbound_buf = queue;

    loop {
        // Read packet from stream
        let (packet, _raw) = match packet::read_packet(client, &mut inbound_buf, &mut reader).await
        {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc", "Closing connection, error occurred");
                break;
            }
        };

        // Grab client state
        let client_state = client.state();

        // Hijack login start
        if client_state == ClientState::Login && packet.id == packets::login::SERVER_LOGIN_START {
            // Parse login start packet
            let login_start = LoginStart::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            debug!(target: "lazymc::lobby", "Login on lobby server (user: {})", login_start.name);

            // Replay Forge payload
            if config.server.forge {
                forge::replay_login_payload(client, &mut inbound, server.clone(), &mut inbound_buf)
                    .await?;
                let (_returned_reader, returned_writer) = inbound.split();
                writer = returned_writer;
            }

            // Respond with set compression if compression is enabled based on threshold
            if proto::COMPRESSION_THRESHOLD >= 0 {
                trace!(target: "lazymc::lobby", "Enabling compression for lobby client because server has it enabled (threshold: {})", proto::COMPRESSION_THRESHOLD);
                respond_set_compression(client, &mut writer, proto::COMPRESSION_THRESHOLD).await?;
                client.set_compression(proto::COMPRESSION_THRESHOLD);
            }

            // Respond with login success, switch to play state
            respond_login_success(client, &mut writer, &login_start).await?;
            client.set_state(ClientState::Play);

            trace!(target: "lazymc::lobby", "Client login success, sending required play packets for lobby world");

            // Send packets to client required to get into workable play state for lobby world
            send_lobby_play_packets(client, &client_info, &mut writer, &server).await?;

            // Wait for server to come online
            stage_wait(client, &client_info, &server, &config, &mut writer).await?;

            // Start new connection to server
            let server_client_info = client_info.clone();
            let (server_client, mut outbound, mut server_buf) =
                connect_to_server(&server_client_info, &inbound, &config).await?;
            let (returned_reader, returned_writer) = inbound.split();
            reader = returned_reader;
            writer = returned_writer;

            // Grab join game packet from server
            let join_game_data = wait_for_server_join_game(
                &server_client,
                &server_client_info,
                &mut outbound,
                &mut server_buf,
            )
            .await?;

            // Reset lobby title
            packets::play::title::send(client, &client_info, &mut writer, "").await?;

            // Play ready sound if configured
            play_lobby_ready_sound(client, &client_info, &mut writer, &config).await?;

            // Wait a second because Notchian servers are slow
            // See: https://wiki.vg/Protocol#Login_Success
            trace!(target: "lazymc::lobby", "Waiting a second before relaying client connection...");
            time::sleep(SERVER_WARMUP).await;

            // Send respawn packet, initiates teleport to real server world
            packets::play::respawn::lobby_send(client, &client_info, &mut writer, join_game_data)
                .await?;

            // Drain inbound connection so we don't confuse the server
            // TODO: can we void everything? we might need to forward everything to server except
            //       for some blacklisted ones
            trace!(target: "lazymc::lobby", "Voiding remaining incoming lobby client data before relay to real server");
            drain_stream(&mut reader).await?;

            // Client and server connection ready now, move client to proxy
            debug!(target: "lazymc::lobby", "Server connection ready, relaying lobby client to proxy");
            route_proxy(inbound, outbound, server_buf);

            return Ok(());
        }

        // Show unhandled packet warning
        debug!(target: "lazymc", "Got unhandled packet:");
        debug!(target: "lazymc", "- State: {:?}", client_state);
        debug!(target: "lazymc", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // Gracefully close connection
    net::close_tcp_stream(inbound).await.map_err(|_| ())?;

    Ok(())
}

/// Respond to client with a set compression packet.
async fn respond_set_compression(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    threshold: i32,
) -> Result<(), ()> {
    packet::write_packet(SetCompression { threshold }, client, writer).await
}

/// Respond to client with login success packet
// TODO: support online mode here
async fn respond_login_success(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    login_start: &LoginStart,
) -> Result<(), ()> {
    packet::write_packet(
        LoginSuccess {
            uuid: uuid::offline_player_uuid(&login_start.name),
            username: login_start.name.clone(),
        },
        client,
        writer,
    )
    .await
}

/// Play lobby ready sound effect if configured.
async fn play_lobby_ready_sound(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    config: &Config,
) -> Result<(), ()> {
    if let Some(sound_name) = config.join.lobby.ready_sound.as_ref() {
        // Must not be empty string
        if sound_name.trim().is_empty() {
            warn!(target: "lazymc::lobby", "Lobby ready sound effect is an empty string, you should remove the configuration item instead");
            return Ok(());
        }

        // Play sound effect
        packets::play::player_pos::send(client, client_info, writer).await?;
        packets::play::sound::send(client, client_info, writer, sound_name).await?;
    }

    Ok(())
}

/// Send packets to client to get workable play state for lobby world.
async fn send_lobby_play_packets(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    server: &Server,
) -> Result<(), ()> {
    // See: https://wiki.vg/Protocol_FAQ#What.27s_the_normal_login_sequence_for_a_client.3F

    // Send initial game join
    packets::play::join_game::lobby_send(client, client_info, writer, server).await?;

    // Send server brand
    packets::play::server_brand::send(client, client_info, writer).await?;

    // Send spawn and player position, disables 'download terrain' screen
    packets::play::player_pos::send(client, client_info, writer).await?;

    // Notify client of world time, required once before keep-alive packets
    packets::play::time_update::send(client, client_info, writer).await?;

    Ok(())
}

/// An infinite keep-alive loop.
///
/// This will keep sending keep-alive and title packets to the client until it is dropped.
async fn keep_alive_loop(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    config: &Config,
) -> Result<(), ()> {
    let mut interval = time::interval(KEEP_ALIVE_INTERVAL);

    loop {
        interval.tick().await;

        trace!(target: "lazymc::lobby", "Sending keep-alive sequence to lobby client");

        // Send keep alive and title packets
        packets::play::keep_alive::send(client, client_info, writer).await?;
        packets::play::title::send(client, client_info, writer, &config.join.lobby.message).await?;

        // TODO: verify we receive correct keep alive response
    }
}

/// Waiting stage.
///
/// In this stage we wait for the server to come online.
///
/// During this stage we keep sending keep-alive and title packets to the client to keep it active.
async fn stage_wait(
    client: &Client,
    client_info: &ClientInfo,
    server: &Server,
    config: &Config,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    select! {
        a = keep_alive_loop(client, client_info, writer, config) => a,
        b = wait_for_server(server, config) => b,
    }
}

/// Wait for the server to come online.
///
/// Returns `Ok(())` once the server is online, returns `Err(())` if waiting failed.
async fn wait_for_server(server: &Server, config: &Config) -> Result<(), ()> {
    debug!(target: "lazymc::lobby", "Waiting on server to come online...");

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
                    trace!(target: "lazymc::lobby", "Server not ready, holding client for longer");
                    continue;
                }

                // Server started, start relaying and proxy
                State::Started => {
                    break true;
                }

                // Server stopping, this shouldn't happen, kick
                State::Stopping | State::Stopped => {
                    break false;
                }
            }
        }
    };

    // Wait for server state with timeout
    let timeout = Duration::from_secs(config.join.lobby.timeout as u64);
    match time::timeout(timeout, task_wait).await {
        // Relay client to proxy
        Ok(true) => {
            debug!(target: "lazymc::lobby", "Server ready for lobby client");
            return Ok(());
        }

        // Server stopping/stopped, this shouldn't happen, disconnect
        Ok(false) => {}

        // Timeout reached, disconnect
        Err(_) => {
            warn!(target: "lazymc::lobby", "Lobby client waiting for server to come online reached timeout of {}s", timeout.as_secs());
        }
    }

    Err(())
}

/// Create connection to the server, with timeout.
///
/// This will initialize the connection to the play state. Client details are used.
async fn connect_to_server(
    client_info: &ClientInfo,
    inbound: &TcpStream,
    config: &Config,
) -> Result<(Client, TcpStream, BytesMut), ()> {
    time::timeout(
        SERVER_CONNECT_TIMEOUT,
        connect_to_server_no_timeout(client_info, inbound, config),
    )
    .await
    .map_err(|_| {
        error!(target: "lazymc::lobby", "Creating new server connection for lobby client timed out after {}s", SERVER_CONNECT_TIMEOUT.as_secs());
    })?
}

/// Create connection to the server, with no timeout.
///
/// This will initialize the connection to the play state. Client details are used.
// TODO: clean this up
async fn connect_to_server_no_timeout(
    client_info: &ClientInfo,
    inbound: &TcpStream,
    config: &Config,
) -> Result<(Client, TcpStream, BytesMut), ()> {
    // Open connection
    // TODO: on connect fail, ping server and redirect to serve_status if offline
    let mut outbound = TcpStream::connect(config.server.address)
        .await
        .map_err(|_| ())?;

    // Add proxy header
    if config.server.send_proxy_v2 {
        trace!(target: "lazymc::lobby", "Sending client proxy header for server connection");
        outbound
            .write_all(&proxy::stream_proxy_header(inbound).map_err(|_| ())?)
            .await
            .map_err(|_| ())?;
    }

    // Construct temporary server client
    let tmp_client = match outbound.local_addr() {
        Ok(addr) => Client::new(addr),
        Err(_) => Client::dummy(),
    };
    tmp_client.set_state(ClientState::Login);

    let (mut reader, mut writer) = outbound.split();

    // Replay client handshake packet
    assert_eq!(
        client_info.handshake.as_ref().unwrap().next_state,
        ClientState::Login.to_id(),
        "Client handshake should have login as next state"
    );
    packet::write_packet(
        client_info.handshake.clone().unwrap(),
        &tmp_client,
        &mut writer,
    )
    .await?;

    // Request login start
    packet::write_packet(
        LoginStart {
            name: client_info.username.clone().ok_or(())?,
        },
        &tmp_client,
        &mut writer,
    )
    .await?;

    // Incoming buffer
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, _raw) = match packet::read_packet(&tmp_client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc::lobby", "Closing connection, error occurred");
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
                    target: "lazymc::lobby",
                    "Compression threshold sent to lobby client does not match threshold from server, this may cause errors (client: {}, server: {})",
                    proto::COMPRESSION_THRESHOLD,
                    set_compression.threshold
                );
            }

            // Set client compression
            tmp_client.set_compression(set_compression.threshold);
            continue;
        }

        // Catch encryption requests
        if client_state == ClientState::Login
            && packet.id == packets::login::CLIENT_ENCRYPTION_REQUEST
        {
            error!(
                target: "lazymc::lobby",
                "Got encryption request from server, this is unsupported. Server must be in offline mode to use lobby.",
            );

            break;
        }

        // Hijack login plugin request
        if client_state == ClientState::Login
            && packet.id == packets::login::CLIENT_LOGIN_PLUGIN_REQUEST
        {
            // Decode login plugin request
            let plugin_request =
                LoginPluginRequest::decode(&mut packet.data.as_slice()).map_err(|err| {
                    dbg!(err);
                })?;

            // Respond with Forge messages
            if config.server.forge {
                trace!(target: "lazymc::lobby", "Got login plugin request from server, responding with Forge reply");

                // Respond to Forge login plugin request
                forge::respond_login_plugin_request(&tmp_client, plugin_request, &mut writer)
                    .await?;

                continue;
            }

            warn!(target: "lazymc::lobby", "Got unexpected login plugin request from server, you may need to enable Forge support");

            // Write unsuccesful login plugin response
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
            trace!(target: "lazymc::lobby", "Got login success from server connection, change to play mode");

            // TODO: parse this packet to ensure it's fine
            // let login_success =
            //     LoginSuccess::decode(&mut packet.data.as_slice()).map_err(|err| {
            //         dbg!(err);
            //         ()
            //     })?;

            // Switch to play state
            tmp_client.set_state(ClientState::Play);

            // Server must enable compression if enabled for client, show warning otherwise
            if tmp_client.is_compressed() != (proto::COMPRESSION_THRESHOLD >= 0) {
                error!(target: "lazymc::lobby", "Compression enabled for lobby client while the server did not, this will cause errors");
            }

            return Ok((tmp_client, outbound, buf));
        }

        // Hijack disconnect
        if client_state == ClientState::Login && packet.id == packets::login::CLIENT_DISCONNECT {
            error!(target: "lazymc::lobby", "Got disconnect from server connection");

            // // Decode disconnect packet
            // let login_disconnect =
            //     LoginDisconnect::decode(&mut packet.data.as_slice()).map_err(|err| {
            //         dbg!(err);
            //     })?;

            // TODO: report/forward error to client

            break;
        }

        // Show unhandled packet warning
        debug!(target: "lazymc::lobby", "Got unhandled packet from server in connect_to_server:");
        debug!(target: "lazymc::lobby", "- State: {:?}", client_state);
        debug!(target: "lazymc::lobby", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
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
        SERVER_JOIN_GAME_TIMEOUT,
        wait_for_server_join_game_no_timeout(client, client_info, outbound, buf),
    )
    .await
    .map_err(|_| {
        error!(target: "lazymc::lobby", "Waiting for for game data from server for lobby client timed out after {}s", SERVER_JOIN_GAME_TIMEOUT.as_secs());
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
                error!(target: "lazymc::lobby", "Closing connection, error occurred");
                break;
            }
        };

        // Catch join game
        if packets::play::join_game::is_packet(client_info, packet.id) {
            // Parse join game data
            let join_game_data = JoinGameData::from_packet(client_info, packet).map_err(|err| {
                warn!(target: "lazymc::lobby", "Failed to parse join game packet: {:?}", err);
            })?;

            return Ok(join_game_data);
        }

        // Show unhandled packet warning
        debug!(target: "lazymc::lobby", "Got unhandled packet from server in wait_for_server_join_game:");
        debug!(target: "lazymc::lobby", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // Gracefully close connection
    net::close_tcp_stream_ref(outbound).await.map_err(|_| ())?;

    Err(())
}

/// Route our lobby client through the proxy to the real server, spawning a new task.
///
/// `inbound_queue` is used for data already received from the server, that needs to be pushed to
/// the client.
#[inline]
pub fn route_proxy(inbound: TcpStream, outbound: TcpStream, inbound_queue: BytesMut) {
    // When server is online, proxy all
    let service = async move {
        proxy::proxy_inbound_outbound_with_queue(inbound, outbound, &inbound_queue, &[])
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "lazymc", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}

/// Drain given reader until nothing is left voiding all data.
async fn drain_stream(reader: &mut ReadHalf<'_>) -> Result<(), ()> {
    let mut drain_buf = [0; 8 * 1024];
    loop {
        match reader.try_read(&mut drain_buf) {
            Ok(0) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
            Ok(_) => continue,
            Err(err) => {
                error!(target: "lazymc::lobby", "Failed to drain lobby client connection before relaying to real server. Maybe already disconnected? Error: {:?}", err);
                return Ok(());
            }
        }
    }
}
