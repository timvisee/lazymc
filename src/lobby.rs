use std::io::ErrorKind;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use futures::FutureExt;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::{LoginStart, LoginSuccess};
use minecraft_protocol::version::v1_17_1::game::{
    ClientBoundKeepAlive, JoinGame, NamedSoundEffect, PlayerPositionAndLook, PluginMessage,
    Respawn, SetTitleSubtitle, SetTitleText, SetTitleTimes, TimeUpdate,
};
use nbt::CompoundTag;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::time;
use uuid::Uuid;

use crate::config::*;
use crate::mc;
use crate::proto::{self, Client, ClientInfo, ClientState, RawPacket};
use crate::proxy;
use crate::server::{Server, State};

// TODO: remove this before releasing feature
pub const USE_LOBBY: bool = true;
pub const DONT_START_SERVER: bool = false;
const STARTING_BANNER: &str = "§2Server is starting\n§7⌛ Please wait...";
const JOIN_SOUND: bool = true;
const JOIN_SOUND_NAME: &str = "block.note_block.chime";

/// Interval to send keep-alive packets at.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Auto incrementing ID source for keep alive packets.
const KEEP_ALIVE_ID: AtomicU64 = AtomicU64::new(0);

/// Lobby clients may wait a maximum of 10 minutes for the server to come online.
const SERVER_WAIT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Time to wait before responding to newly connected server.
///
/// Notchian servers are slow, we must wait a little before sending play packets, because the
/// server needs time to transition the client into this state.
/// See warning at: https://wiki.vg/Protocol#Login_Success
const SERVER_WARMUP: Duration = Duration::from_secs(1);

/// Server brand to send to client in lobby world.
///
/// Shown in F3 menu. Updated once client is relayed to real server.
const SERVER_BRAND: &[u8] = b"lazymc";

/// Serve lobby service for given client connection.
///
/// The client must be in the login state, or this will error.
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
        let (packet, _raw) = match proto::read_packet(&mut inbound_buf, &mut reader).await {
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
        if client_state == ClientState::Login
            && packet.id == proto::packets::login::SERVER_LOGIN_START
        {
            // Parse login start packet
            let login_start = LoginStart::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            debug!(target: "lazymc::lobby", "Login on lobby server (user: {})", login_start.name);

            // Respond with login success, switch to play state
            respond_login_success(&mut writer, &login_start).await?;
            client.set_state(ClientState::Play);

            trace!(target: "lazymc::lobby", "Client login success, sending required play packets for lobby world");

            // Send packets to client required to get into workable play state for lobby world
            send_lobby_play_packets(&mut writer, &server).await?;

            // Wait for server to come online, then set up new connection to it
            stage_wait(server.clone(), &mut writer).await?;
            let (mut outbound, mut server_buf) = connect_to_server(client_info, &config).await?;

            // Grab join game packet from server
            let join_game = wait_for_server_join_game(&mut outbound, &mut server_buf).await?;

            // Reset lobby title, play sound effect
            send_lobby_title(&mut writer, "").await?;
            if JOIN_SOUND {
                send_lobby_sound_effect(&mut writer).await?;
            }

            // Wait a second because Notchian servers are slow
            // See: https://wiki.vg/Protocol#Login_Success
            trace!(target: "lazymc::lobby", "Waiting a second before relaying client connection...");
            time::sleep(SERVER_WARMUP).await;

            // Send respawn packet, initiates teleport to real server world
            send_respawn_from_join(&mut writer, join_game).await?;

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

/// Respond to client with login success packet
// TODO: support online mode here
async fn respond_login_success(
    writer: &mut WriteHalf<'_>,
    login_start: &LoginStart,
) -> Result<(), ()> {
    let packet = LoginSuccess {
        uuid: Uuid::new_v3(
            &Uuid::new_v3(&Uuid::nil(), b"OfflinePlayer"),
            login_start.name.as_bytes(),
        ),
        username: login_start.name.clone(),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::login::CLIENT_LOGIN_SUCCESS, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send packets to client to get workable play state for lobby world.
async fn send_lobby_play_packets(writer: &mut WriteHalf<'_>, server: &Server) -> Result<(), ()> {
    // See: https://wiki.vg/Protocol_FAQ#What.27s_the_normal_login_sequence_for_a_client.3F

    // Send initial game join
    send_lobby_join_game(writer, server).await?;

    // Send server brand
    send_lobby_brand(writer).await?;

    // Send spawn and player position, disables 'download terrain' screen
    // Note: If this screen stays, we may need to send it a second time
    send_lobby_player_pos(writer).await?;

    // Notify client of world time, required once before keep-alive packets
    send_lobby_time_update(writer).await?;

    Ok(())
}

/// Send initial join game packet to client for lobby.
async fn send_lobby_join_game(writer: &mut WriteHalf<'_>, server: &Server) -> Result<(), ()> {
    // Send Minecrafts default states, slightly customised for lobby world
    let packet = {
        let status = server.status();

        JoinGame {
            // Player ID must be unique, if it collides with another server entity ID the player gets
            // in a weird state and cannot move
            entity_id: 0,
            // TODO: use real server value
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
            world_name: "lazymc:lobby".into(),
            hashed_seed: 0,
            max_players: status.as_ref().map(|s| s.players.max as i32).unwrap_or(20),
            // TODO: use real server value
            view_distance: 10,
            // TODO: use real server value
            reduced_debug_info: false,
            // TODO: use real server value
            enable_respawn_screen: true,
            is_debug: true,
            is_flat: false,
        }
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_JOIN_GAME, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send lobby brand to client.
async fn send_lobby_brand(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let packet = PluginMessage {
        channel: "minecraft:brand".into(),
        data: SERVER_BRAND.into(),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_PLUGIN_MESSAGE, data).encode()?;
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
    let packet = ClientBoundKeepAlive {
        // Keep sending new IDs
        id: KEEP_ALIVE_ID.fetch_add(1, Ordering::Relaxed),
    };

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
        // Defaults: https://minecraft.fandom.com/wiki/Commands/title#Detail
        SetTitleTimes {
            fade_in: 10,
            stay: 70,
            fade_out: 20,
        }
    } else {
        SetTitleTimes {
            fade_in: 0,
            stay: KEEP_ALIVE_INTERVAL.as_secs() as i32 * mc::TICKS_PER_SECOND as i32 * 2,
            fade_out: 0,
        }
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_TIMES, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

/// Send lobby ready sound effect to client.
async fn send_lobby_sound_effect(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let packet = NamedSoundEffect {
        sound_name: JOIN_SOUND_NAME.into(),
        sound_category: 0,
        effect_pos_x: 0,
        effect_pos_y: 0,
        effect_pos_z: 0,
        volume: 1.0,
        pitch: 1.0,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response =
        RawPacket::new(proto::packets::play::CLIENT_NAMED_SOUND_EFFECT, data).encode()?;
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
async fn stage_wait<'a>(server: Arc<Server>, writer: &mut WriteHalf<'a>) -> Result<(), ()> {
    select! {
        a = keep_alive_loop(writer) => a,
        b = wait_for_server(server) => b,
    }
}

/// Wait for the server to come online.
///
/// Returns `Ok(())` once the server is online, returns `Err(())` if waiting failed.
async fn wait_for_server<'a>(server: Arc<Server>) -> Result<(), ()> {
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
    match time::timeout(SERVER_WAIT_TIMEOUT, task_wait).await {
        // Relay client to proxy
        Ok(true) => {
            debug!(target: "lazymc::lobby", "Server ready for lobby client");
            return Ok(());
        }

        // Server stopping/stopped, this shouldn't happen, disconnect
        Ok(false) => {}

        // Timeout reached, disconnect
        Err(_) => {
            warn!(target: "lazymc::lobby", "Lobby client waiting for server to come online reached timeout of {}s", SERVER_WAIT_TIMEOUT.as_secs());
        }
    }

    Err(())
}

/// Create connection to the server.
///
/// This will initialize the connection to the play state. Client details are used.
// TODO: clean this up
// TODO: add timeout
async fn connect_to_server(
    client_info: ClientInfo,
    config: &Config,
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

    let request = RawPacket::new(proto::packets::handshake::SERVER_HANDSHAKE, data).encode()?;
    writer.write_all(&request).await.map_err(|_| ())?;

    // Request login start
    let packet = LoginStart {
        name: client_info.username.ok_or(())?,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let request = RawPacket::new(proto::packets::login::SERVER_LOGIN_START, data).encode()?;
    writer.write_all(&request).await.map_err(|_| ())?;

    // Incoming buffer
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, _raw) = match proto::read_packet(&mut buf, &mut reader).await {
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
        if client_state == ClientState::Login
            && packet.id == proto::packets::login::CLIENT_LOGIN_SUCCESS
        {
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

    // Gracefully close connection
    match writer.shutdown().await {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotConnected => {}
        Err(_) => return Err(()),
    }

    Err(())
}

/// Wait for join game packet on server connection.
///
/// This parses, consumes and returns the packet.
// TODO: clean this up
// TODO: do not drop error here, return Box<dyn Error>
// TODO: add timeout
async fn wait_for_server_join_game(
    outbound: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<JoinGame, ()> {
    let (mut reader, mut writer) = outbound.split();

    loop {
        // Read packet from stream
        let (packet, _raw) = match proto::read_packet(buf, &mut reader).await {
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
async fn drain_stream<'a>(reader: &mut ReadHalf<'a>) -> Result<(), ()> {
    let mut drain_buf = [0; 8 * 1024];
    loop {
        match reader.try_read(&mut drain_buf) {
            Ok(read) if read == 0 => return Ok(()),
            Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
            Ok(_) => continue,
            Err(err) => {
                error!(target: "lazymc::lobby", "Failed to drain lobby client connection before relaying to real server. Maybe already disconnected? Error: {:?}", err);
                return Ok(());
            }
        }
    }
}

/// Read NBT CompoundTag from SNBT.
fn snbt_to_compound_tag(data: &str) -> CompoundTag {
    use nbt::decode::read_compound_tag;
    use quartz_nbt::io::{write_nbt, Flavor};
    use quartz_nbt::snbt;

    // Parse SNBT data
    let compound = snbt::parse(data).expect("failed to parse SNBT");

    // Encode to binary
    let mut binary = Vec::new();
    write_nbt(&mut binary, None, &compound, Flavor::Uncompressed)
        .expect("failed to encode NBT CompoundTag as binary");

    // Parse binary with usable NBT create
    read_compound_tag(&mut &*binary).unwrap()
}
