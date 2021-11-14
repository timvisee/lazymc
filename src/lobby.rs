// TODO: remove this before feature release!
#![allow(unused)]

use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use futures::FutureExt;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::data::server_status::*;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::game::{GameMode, MessagePosition};
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::{LoginDisconnect, LoginStart, LoginSuccess};
use minecraft_protocol::version::v1_14_4::status::StatusResponse;
use minecraft_protocol::version::v1_17_1::game::{
    ChunkData, ClientBoundChatMessage, ClientBoundKeepAlive, GameDisconnect, JoinGame,
    PlayerPositionAndLook, PluginMessage, Respawn, SetTitleSubtitle, SetTitleText, SetTitleTimes,
    SpawnPosition, TimeUpdate,
};
use nbt::CompoundTag;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::time;
use uuid::Uuid;

use crate::config::*;
use crate::proto::{self, Client, ClientInfo, ClientState, RawPacket};
use crate::proxy;
use crate::server::{self, Server, State};
use crate::service;

// TODO: remove this before releasing feature
pub const USE_LOBBY: bool = true;
pub const DONT_START_SERVER: bool = false;
const STARTING_BANNER: &str = "§2Server is starting\n§7⌛ Please wait...";
const HOLD_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Interval for server state polling when waiting on server to come online.
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Interval to send keep-alive packets at.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Minecraft ticks per second.
const TICKS_PER_SECOND: u32 = 20;

// TODO: do not drop error here, return Box<dyn Error>
// TODO: on error, nicely kick client with message
pub async fn serve(
    client: Client,
    client_info: ClientInfo,
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
    queue: BytesMut,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // TODO: note this assumes the first receiving packet (over queue) is login start
    // TODO: assert client is in login mode!

    // We must have useful client info
    if client_info.username.is_none() {
        error!(target: "lazymc::lobby", "Client username is unknown, closing connection");
        return Err(());
    }

    // Incoming buffer and packet holding queue
    let mut inbound_buf = queue;
    let mut server_queue = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, raw) = match proto::read_packet(&mut inbound_buf, &mut reader).await {
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
        if client_state == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_START {
            // Parse login start packet
            let login_start = LoginStart::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            debug!(target: "lazymc::lobby", "Login on lobby server (user: {})", login_start.name);

            // Respond with login success, switch to play state
            respond_login_success(&mut writer, &login_start).await?;
            client.set_state(ClientState::Play);

            trace!(target: "lazymc::lobby", "Client login success, sending required play packets for lobby world");

            // Send packets to client required to get into workable play state for lobby world
            send_lobby_play_packets(&mut writer).await;

            // Wait for server to come online, then set up new connection to it
            stage_wait(config.clone(), server.clone(), &mut writer).await?;
            let (mut outbound, mut server_buf) =
                connect_to_server(client, client_info, &config, server).await?;

            // Grab join game packet from server
            let join_game = wait_for_server_join_game(&mut outbound, &mut server_buf).await?;

            // TODO: we might have excess server_buf data here, do something with it!
            if !server_buf.is_empty() {
                error!(target: "lazymc::lobby", "Got excess data from server for client, throwing it away ({} bytes)", server_buf.len());
                // TODO: remove after debug
                dbg!(server_buf);
            }

            // Reset our lobby title
            send_lobby_title(&mut writer, "").await?;

            // Send respawn packet, initiates teleport to real server world
            // TODO: should we just send one packet?
            send_respawn_from_join(&mut writer, join_game.clone()).await?;
            send_respawn_from_join(&mut writer, join_game).await?;

            // Drain inbound connection so we don't confuse the server
            // TODO: can we drain everything? we might need to forward everything to server except
            //       for some blacklisted ones
            drain_stream(&mut reader).await?;

            // TODO: should we wait a little?
            // Wait a little because Notchian servers are slow
            // See: https://wiki.vg/Protocol#Login_Success
            // trace!(target: "lazymc::lobby", "Waiting a second before relaying client connection...");
            // time::sleep(Duration::from_secs(1)).await;

            // Client and server connection ready now, move client to proxy
            debug!(target: "lazymc::lobby", "Server connection ready, moving client to proxy");
            route_proxy(inbound, outbound, config);

            return Ok(());
        }

        // TODO: when receiving Login Plugin Request, respond with empty payload
        // See: https://wiki.vg/Protocol#Login_Plugin_Request

        // Show unhandled packet warning
        debug!(target: "lazymc", "Received unhandled packet:");
        debug!(target: "lazymc", "- State: {:?}", client_state);
        debug!(target: "lazymc", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // Gracefully close connection
    match writer.shutdown().await {
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

    let response = RawPacket::new(proto::LOGIN_PACKET_ID_DISCONNECT, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())
}

/// Respond to client with login success packet
// TODO: support online mode here
async fn respond_login_success(
    writer: &mut WriteHalf<'_>,
    login_start: &LoginStart,
) -> Result<(), ()> {
    let packet = LoginSuccess {
        uuid: Uuid::new_v3(
            // TODO: use Uuid::null() here as namespace?
            &Uuid::new_v3(&Uuid::NAMESPACE_OID, b"OfflinePlayer"),
            login_start.name.as_bytes(),
        ),
        username: login_start.name.clone(),
        // uuid: Uuid::parse_str("35ee313b-d89a-41b8-b25e-d32e8aff0389").unwrap(),
        // username: "Username".into(),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::LOGIN_PACKET_ID_LOGIN_SUCCESS, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send packets to client to get workable play state for lobby world.
async fn send_lobby_play_packets(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // See: https://wiki.vg/Protocol_FAQ#What.27s_the_normal_login_sequence_for_a_client.3F

    // Send initial game join
    send_lobby_join_game(writer).await?;

    // Send server brand
    // TODO: does client ever receive real brand after this?
    send_lobby_brand(writer).await?;

    // Send spawn and player position, disables 'download terrain' screen
    // TODO: is sending spawn this required?
    send_lobby_spawn_pos(writer).await?;
    send_lobby_player_pos(writer).await?;

    // Notify client of world time, required once before keep-alive packets
    send_lobby_time_update(writer).await?;

    // TODO: we might need to send player_pos one more time

    Ok(())
}

/// Send initial join game packet to client for lobby.
async fn send_lobby_join_game(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // Send Minecrafts default states, slightly customised for lobby world
    // TODO: use values from real server here!
    let packet = JoinGame {
        // TODO: ID must not collide with any other entity, possibly send huge number
        entity_id: 0,
        hardcore: false,
        game_mode: 3,
        previous_game_mode: -1i8 as u8,
        world_names: vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ],
        dimension_codec: snbt_to_compound_tag(include_str!("../res/dimension_codec.snbt")),
        dimension: snbt_to_compound_tag(include_str!("../res/dimension.snbt")),
        // TODO: test whether using minecraft:overworld breaks?
        world_name: "lazymc:lobby".into(),
        hashed_seed: 0,
        max_players: 20,
        // TODO: try very low view distance?
        view_distance: 10,
        // TODO: set to true!
        reduced_debug_info: false,
        enable_respawn_screen: false,
        is_debug: true,
        is_flat: false,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_JOIN_GAME, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send lobby brand to client.
async fn send_lobby_brand(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let brand = b"lazymc".to_vec();

    let packet = PluginMessage {
        channel: "minecraft:brand".into(),
        data: brand,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_PLUGIN_MESSAGE, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send lobby spawn position to client.
async fn send_lobby_spawn_pos(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let packet = SpawnPosition {
        position: 0,
        angle: 0.0,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SPAWN_POS, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send lobby player position to client.
async fn send_lobby_player_pos(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // Send player location, disables download terrain screen
    let packet = PlayerPositionAndLook {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        yaw: 0.0,
        pitch: 90.0,
        flags: 0b00000000,
        teleport_id: 0,
        dismount_vehicle: true,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_PLAYER_POS_LOOK, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send lobby time update to client.
async fn send_lobby_time_update(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    const MC_TIME_NOON: i64 = 6000;

    // Send time update, required once for keep-alive packets
    let packet = TimeUpdate {
        world_age: MC_TIME_NOON,
        time_of_day: MC_TIME_NOON,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_TIME_UPDATE, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send keep alive packet to client.
///
/// Required periodically in play mode to prevent client timeout.
async fn send_keep_alive(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // TODO: keep picking random ID!
    let packet = ClientBoundKeepAlive { id: 0 };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_KEEP_ALIVE, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // TODO: verify we receive keep alive response with same ID from client

    Ok(())
}

/// Send lobby title packets to client.
///
/// This will show the given text for two keep-alive periods. Use a newline for the subtitle.
///
/// If an empty string is given, the title times will be reset to default.
async fn send_lobby_title(writer: &mut WriteHalf<'_>, text: &str) -> Result<(), ()> {
    // Grab title and subtitle bits
    let title = text.lines().next().unwrap_or("");
    let subtitle = text.lines().skip(1).collect::<Vec<_>>().join("\n");

    // Set title
    let packet = SetTitleText {
        text: Message::new(Payload::text(title)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_TEXT, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // Set subtitle
    let packet = SetTitleSubtitle {
        text: Message::new(Payload::text(&subtitle)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response =
        RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_SUBTITLE, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // Set title times
    let packet = if title.is_empty() && subtitle.is_empty() {
        // TODO: figure out real default values here
        SetTitleTimes {
            fade_in: 10,
            stay: 100,
            fade_out: 10,
        }
    } else {
        SetTitleTimes {
            fade_in: 0,
            stay: KEEP_ALIVE_INTERVAL.as_secs() as i32 * TICKS_PER_SECOND as i32 * 2,
            fade_out: 0,
        }
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_TIMES, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send respawn packet to client to jump from lobby into now loaded server.
///
/// The required details will be fetched from the `join_game` packet as provided by the server.
async fn send_respawn_from_join(writer: &mut WriteHalf<'_>, join_game: JoinGame) -> Result<(), ()> {
    let packet = Respawn {
        dimension: join_game.dimension,
        world_name: join_game.world_name,
        hashed_seed: join_game.hashed_seed,
        game_mode: join_game.game_mode,
        previous_game_mode: join_game.previous_game_mode,
        is_debug: join_game.is_debug,
        is_flat: join_game.is_flat,
        copy_metadata: false,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_RESPAWN, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// An infinite keep-alive loop.
///
/// This will keep sending keep-alive and title packets to the client until it is dropped.
async fn keep_alive_loop(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let mut interval = time::interval(KEEP_ALIVE_INTERVAL);

    loop {
        interval.tick().await;

        trace!(target: "lazymc::lobby", "Sending keep-alive sequence to lobby client");

        // Send keep alive and title packets
        send_keep_alive(writer).await?;
        send_lobby_title(writer, STARTING_BANNER).await?;
    }
}

/// Waiting stage.
///
/// In this stage we wait for the server to come online.
///
/// During this stage we keep sending keep-alive and title packets to the client to keep it active.
// TODO: should we use some timeout in here, could be large?
async fn stage_wait<'a>(
    config: Arc<Config>,
    server: Arc<Server>,
    writer: &mut WriteHalf<'a>,
) -> Result<(), ()> {
    // Ensure server poll interval is less than keep alive interval
    // We need to wait on the smallest interval in the following loop
    assert!(
        SERVER_POLL_INTERVAL <= KEEP_ALIVE_INTERVAL,
        "SERVER_POLL_INTERVAL should be <= KEEP_ALIVE_INTERVAL"
    );

    select! {
        a = keep_alive_loop(writer) => a,
        b = wait_for_server(config, server) => b,
    }
}

/// Wait for the server to come online.
///
/// Returns `Ok(())` once the server is online, returns `Err(())` if waiting failed.
// TODO: go through this, use proper error messages
async fn wait_for_server<'a>(config: Arc<Config>, server: Arc<Server>) -> Result<(), ()> {
    debug!(target: "lazymc::lobby", "Waiting on server...");

    // Set up polling interval, get timeout
    let mut poll_interval = time::interval(HOLD_POLL_INTERVAL);
    let since = Instant::now();
    let timeout = config.time.hold_client_for as u64;

    loop {
        // TODO: wait for start signal over channel instead of polling
        poll_interval.tick().await;

        trace!(target: "lazymc::lobby", "Polling outbound server state for lobby client...");

        match server.state() {
            // Still waiting on server start
            State::Starting => {
                trace!(target: "lazymc::lobby", "Server not ready, holding client for longer");

                // TODO: add timeout here?

                continue;
            }

            // Server started, start relaying and proxy
            State::Started => {
                // TODO: drop client if already disconnected

                debug!(target: "lazymc::lobby", "Server ready for lobby client!");
                return Ok(());
            }

            // Server stopping or stopped, this shouldn't happen
            State::Stopping | State::Stopped => {
                break;
            }
        }
    }

    Err(())
}

/// Create connection to the server.
///
/// This will initialize the connection to the play state. Client details are used.
// TODO: clean this up
async fn connect_to_server(
    real_client: Client,
    client_info: ClientInfo,
    config: &Config,
    server: Arc<Server>,
) -> Result<(TcpStream, BytesMut), ()> {
    // Open connection
    // TODO: on connect fail, ping server and redirect to serve_status if offline
    let mut outbound = TcpStream::connect(config.server.address)
        .await
        .map_err(|_| ())?;

    let (mut reader, mut writer) = outbound.split();

    let tmp_client = Client::default();
    tmp_client.set_state(ClientState::Login);

    // Handshake packet
    let packet = Handshake {
        protocol_version: client_info.protocol_version.unwrap(),
        server_addr: config.server.address.ip().to_string(),
        server_port: config.server.address.port(),
        next_state: ClientState::Login.to_id(),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let request = RawPacket::new(proto::HANDSHAKE_PACKET_ID_HANDSHAKE, data).encode()?;
    writer.write_all(&request).await.map_err(|_| ())?;

    // Request login start
    let packet = LoginStart {
        name: client_info.username.ok_or(())?,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let request = RawPacket::new(proto::LOGIN_PACKET_ID_LOGIN_START, data).encode()?;
    writer.write_all(&request).await.map_err(|_| ())?;

    // Incoming buffer
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, raw) = match proto::read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc::lobby", "Closing connection, error occurred");
                break;
            }
        };

        // Grab client state
        let client_state = tmp_client.state();

        // Hijack login success
        if client_state == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_SUCCESS {
            trace!(target: "lazymc::lobby", "Received login success from server connection, change to play mode");

            // TODO: parse this packet to ensure it's fine
            // let login_success =
            //     LoginSuccess::decode(&mut packet.data.as_slice()).map_err(|err| {
            //         dbg!(err);
            //         ()
            //     })?;

            // Switch to play state
            tmp_client.set_state(ClientState::Play);

            return Ok((outbound, buf));
        }

        // Show unhandled packet warning
        debug!(target: "lazymc::lobby", "Received unhandled packet from server in connect_to_server:");
        debug!(target: "lazymc::lobby", "- State: {:?}", client_state);
        debug!(target: "lazymc::lobby", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // // Gracefully close connection
    // match writer.shutdown().await {
    //     Ok(_) => {}
    //     Err(err) if err.kind() == io::ErrorKind::NotConnected => {}
    //     Err(_) => return Err(()),
    // }

    // TODO: do we ever reach this?
    Err(())
}

/// Wait for join game packet on server connection.
///
/// This parses, consumes and returns the packet.
// TODO: clean this up
// TODO: do not drop error here, return Box<dyn Error>
// TODO: add timeout
async fn wait_for_server_join_game(
    mut outbound: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<JoinGame, ()> {
    let (mut reader, mut writer) = outbound.split();

    loop {
        // Read packet from stream
        let (packet, raw) = match proto::read_packet(buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc::lobby", "Closing connection, error occurred");
                break;
            }
        };

        // Catch join game
        if packet.id == proto::packets::play::CLIENT_JOIN_GAME {
            let join_game = JoinGame::decode(&mut packet.data.as_slice()).map_err(|err| {
                // TODO: remove this debug
                dbg!(err);
                ()
            })?;

            return Ok(join_game);
        }

        // Show unhandled packet warning
        debug!(target: "lazymc::lobby", "Received unhandled packet from server in wait_for_server_join_game:");
        debug!(target: "lazymc::lobby", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // Gracefully close connection
    match writer.shutdown().await {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotConnected => {}
        Err(_) => return Err(()),
    }

    // TODO: will we ever reach this?
    Err(())
}

/// Route our lobby client through the proxy to the real server, spawning a new task.
#[inline]
pub fn route_proxy(inbound: TcpStream, outbound: TcpStream, config: Arc<Config>) {
    // When server is online, proxy all
    let service = async move {
        proxy::proxy_inbound_outbound_with_queue(inbound, outbound, &[], &[])
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "lazymc", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}

/// Drain given reader until nothing is left.
// TODO: go through this, use proper error messages
async fn drain_stream<'a>(reader: &mut ReadHalf<'a>) -> Result<(), ()> {
    // TODO: remove after debug
    trace!(target: "lazymc::lobby", "Draining stream...");

    // TODO: use other size, look at default std::io size?
    let mut drain_buf = [0; 1024];

    loop {
        match reader.try_read(&mut drain_buf) {
            // TODO: stop if read < drain_buf.len() ?
            Ok(read) if read == 0 => return Ok(()),
            Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
            Ok(read) => continue,
            Err(err) => {
                // TODO: remove after debug
                dbg!("drain err", err);
                return Ok(());
            }
        }
    }
}

/// Read NBT CompoundTag from SNBT.
fn snbt_to_compound_tag(data: &str) -> CompoundTag {
    use nbt::decode::read_compound_tag;
    use quartz_nbt::io::{self, Flavor};
    use quartz_nbt::snbt;
    use std::io::Cursor;

    // Parse SNBT data
    let compound = snbt::parse(data).expect("failed to parse SNBT");

    // Encode to binary
    let mut binary = Vec::new();
    io::write_nbt(&mut binary, None, &compound, Flavor::Uncompressed);

    // Parse binary with usable NBT create
    read_compound_tag(&mut &*binary).unwrap()
}
