// TODO: remove this before feature release!
#![allow(unused)]

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
    PlayerPositionAndLook, Respawn, SetTitleSubtitle, SetTitleText, SetTitleTimes, SpawnPosition,
    TimeUpdate,
};
use nbt::CompoundTag;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::time;
use uuid::Uuid;

use crate::config::*;
use crate::proto::{self, Client, ClientState, RawPacket};
use crate::proxy;
use crate::server::{self, Server, State};
use crate::service;

// TODO: remove this before releasing feature
pub const USE_LOBBY: bool = true;
pub const DONT_START_SERVER: bool = false;
const STARTING_BANNER: &str = "§2Server is starting";
const STARTING_BANNER_SUB: &str = "§7⌛ Please wait...";

/// Client holding server state poll interval.
const HOLD_POLL_INTERVAL: Duration = Duration::from_secs(1);

// TODO: do not drop error here, return Box<dyn Error>
pub async fn serve(
    client: Client,
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
    queue: BytesMut,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // TODO: note this assumes the first receiving packet (over queue) is login start
    // TODO: assert client is in login mode!

    // Incoming buffer and packet holding queue
    let mut buf = queue;
    let mut server_queue = BytesMut::new();

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

        // Hijack login start
        if client_state == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_START {
            // Try to get login username
            let login_start = LoginStart::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            // TODO: remove debug message
            debug!(target: "LOBBY", "Login {:?}", login_start.name);

            // Respond with login success
            let packet = LoginSuccess {
                // TODO: use correct username here
                uuid: Uuid::new_v3(
                    &Uuid::new_v3(&Uuid::NAMESPACE_OID, b"OfflinePlayer"),
                    login_start.name.as_bytes(),
                ),
                username: login_start.name,
                // uuid: Uuid::parse_str("35ee313b-d89a-41b8-b25e-d32e8aff0389").unwrap(),
                // username: "Username".into(),
            };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(proto::LOGIN_PACKET_ID_LOGIN_SUCCESS, data).encode()?;
            writer.write_all(&response).await.map_err(|_| ())?;

            // Update client state to play
            client.set_state(ClientState::Play);

            // TODO: remove debug message
            debug!(target: "LOBBY", "Sent login success, moving to play state");

            // TODO: handle errors here
            play_packets(&mut writer).await;

            send_keep_alive(&mut writer).await?;

            send_title(&mut writer).await?;

            // Wait for server to come online
            wait_for_server(config.clone(), server.clone(), &mut writer).await?;

            // Connect to server
            let (mut outbound, client_queue) = connect_to_server(client, &config, server).await?;

            // Wait for join game packet
            // TODO: do something with this excess buffer
            let (join_game, client_buf) = wait_for_server_join_game(&mut outbound).await?;

            dbg!(join_game.entity_id);

            send_title_reset(&mut writer).await?;

            // Send respawn apcket
            send_respawn(&mut writer, join_game.clone()).await?;
            send_respawn(&mut writer, join_game).await?;

            if !client_buf.is_empty() {
                // TODO: handle this
                // TODO: remove error message
                error!(target: "LOBBY", "Got excess data from server for client! ({} bytes)", client_buf.len());
            }

            // Drain inbound
            // TODO: do not drain, send packets to server, except keep-alive
            drain_stream(&mut reader).await?;

            // TODO: route any following packets to client

            // Wait a little because Notchian servers are slow
            // See: https://wiki.vg/Protocol#Login_Success
            // TODO: improve this

            // debug!(target: "LOBBY", "Waiting 2 sec for server");
            // time::sleep(Duration::from_secs(2)).await;

            debug!(target: "LOBBY", "Moving client to proxy");
            route_proxy_queue(inbound, outbound, config, client_queue, server_queue);

            return Ok(());
        }

        if client_state == ClientState::Play
            && packet.id == proto::packets::play::SERVER_CLIENT_SETTINGS
        {
            debug!(target: "LOBBY", "Ignoring client settings packet");
            continue;
        }

        if client_state == ClientState::Play
            && packet.id == proto::packets::play::SERVER_PLUGIN_MESSAGE
        {
            debug!(target: "LOBBY", "Ignoring plugin message packet");
            continue;
        }

        if client_state == ClientState::Play
            && packet.id == proto::packets::play::SERVER_PLAYER_POS_ROT
        {
            debug!(target: "LOBBY", "Ignoring player pos rot packet");
            continue;
        }

        if client_state == ClientState::Play && packet.id == proto::packets::play::SERVER_PLAYER_POS
        {
            debug!(target: "LOBBY", "Ignoring player pos packet");
            continue;
        }

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

async fn play_packets(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    debug!(target: "LOBBY", "Send play packets");

    // See: https://wiki.vg/Protocol_FAQ#What.27s_the_normal_login_sequence_for_a_client.3F

    // Send game join
    send_join_game(writer).await?;

    // TODO: send brand plugin message

    // After this, we receive:
    // - PLAY_PAKCET_ID_CLIENT_SETTINGS
    // - PLAY_PAKCET_ID_PLUGIN_MESSAGE
    // - PLAY_PAKCET_ID_PLAYER_POS_ROT
    // - PLAY_PAKCET_ID_PLAYER_POS ...

    // TODO: send Update View Position ?
    // TODO: send Update View Distance ?

    // Send chunk data
    // TODO: send_chunk_data(writer).await?;

    // TODO: probably not required
    send_spawn_pos(writer).await?;

    // Send player location, disables download terrain screen
    send_player_pos(writer).await?;

    // TODO: send Update View Position
    // TODO: send Spawn Position
    // TODO: send Position and Look (one more time)

    // Send time update
    send_time_update(writer).await?;

    // Keep sending keep alive packets
    send_keep_alive(writer).await?;

    Ok(())
}

async fn send_join_game(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // // TODO: use proper values here!
    // let packet = JoinGame {
    //     // entity_id: 0,
    //     // game_mode: GameMode::Spectator,
    //     entity_id: 27,
    //     game_mode: GameMode::Hardcore,
    //     dimension: 23,
    //     max_players: 100,
    //     level_type: String::from("default"),
    //     view_distance: 10,
    //     reduced_debug_info: true,
    // };

    // TODO: use proper values here!
    let packet = JoinGame {
        // entity_id: 0x6d,
        // ID must not collide with anything existing
        // entity_id: 1337133700,
        entity_id: 0,
        hardcore: false,
        game_mode: 3,
        previous_game_mode: -1i8 as u8, // use -1i8 as u8?
        world_names: vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ],
        dimension_codec: snbt_to_compound_tag(include_str!("../res/dimension_codec.snbt")),
        dimension: snbt_to_compound_tag(include_str!("../res/dimension.snbt")),
        // TODO: is this ok?
        // world_name: "minecraft:overworld".into(),
        world_name: "lazymc:lobby".into(),
        hashed_seed: 0,
        max_players: 20,
        view_distance: 10,
        reduced_debug_info: true,
        enable_respawn_screen: false,
        is_debug: false,
        is_flat: false,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_JOIN_GAME, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

async fn send_respawn(writer: &mut WriteHalf<'_>, join_game: JoinGame) -> Result<(), ()> {
    // TODO: use proper values here!
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

// // TODO: this is possibly broken?
// async fn send_chunk_data(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
//     // Send player location, disables download terrain screen
//     let packet = ChunkData {
//         x: 0,
//         z: 0,
//         primary_mask: Vec::new(),
//         heightmaps: CompoundTag::named("HeightMaps"),
//         biomes: Vec::new(),
//         data_size: 0,
//         data: Vec::new(),
//         block_entities_size: 0,
//         block_entities: Vec::new(),
//         // primary_mask: 65535,
//         // heights: CompoundTag::named("HeightMaps"),
//         // data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
//         // tiles: vec![CompoundTag::named("TileEntity")],
//     };

//     let mut data = Vec::new();
//     packet.encode(&mut data).map_err(|_| ())?;

//     let response = RawPacket::new(proto::CLIENT_CHUNK_DATA, data).encode()?;
//     writer.write_all(&response).await.map_err(|_| ())?;

//     Ok(())
// }

async fn send_spawn_pos(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
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

async fn send_player_pos(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
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

async fn send_time_update(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    const MC_TIME_NOON: i64 = 6000;

    // Send player location, disables download terrain screen
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

async fn send_keep_alive(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // Send player location, disables download terrain screen
    // TODO: keep picking random ID!
    let packet = ClientBoundKeepAlive { id: 0 };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_KEEP_ALIVE, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // TODO: require to receive correct keepalive!

    Ok(())
}

async fn send_keep_alive_loop(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // TODO: use interval of 10 sec?
    let mut poll_interval = time::interval(Duration::from_secs(10));

    loop {
        // TODO: wait for start signal over channel instead of polling
        poll_interval.tick().await;

        debug!(target: "LOBBY", "Sending keep-alive to client");
        send_keep_alive(writer).await?;
    }

    Ok(())
}

async fn send_title(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // Set title
    let packet = SetTitleText {
        text: Message::new(Payload::text(STARTING_BANNER)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_TEXT, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // Set subtitle
    let packet = SetTitleSubtitle {
        text: Message::new(Payload::text(STARTING_BANNER_SUB)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response =
        RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_SUBTITLE, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // Set times
    // TODO: do not make this longer than 2x keep alive packet interval
    let packet = SetTitleTimes {
        fade_in: 0,
        stay: i32::MAX,
        fade_out: 0,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_TIMES, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

async fn send_title_reset(writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    // Set title
    let packet = SetTitleText {
        text: Message::new(Payload::text("")),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_TEXT, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // Set subtitle
    let packet = SetTitleSubtitle {
        text: Message::new(Payload::text("")),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response =
        RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_SUBTITLE, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    // Set times
    // TODO: do not make this longer than 2x keep alive packet interval
    let packet = SetTitleTimes {
        fade_in: 10,
        stay: 100,
        fade_out: 10,
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(proto::packets::play::CLIENT_SET_TITLE_TIMES, data).encode()?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}

// TODO: go through this, use proper error messages
pub async fn wait_for_server<'a>(
    config: Arc<Config>,
    server: Arc<Server>,
    writer: &mut WriteHalf<'a>,
) -> Result<(), ()> {
    debug!(target: "lazymc", "Waiting on server...");

    // Set up polling interval, get timeout
    let mut poll_interval = time::interval(HOLD_POLL_INTERVAL);
    let since = Instant::now();
    let timeout = config.time.hold_client_for as u64;

    loop {
        // TODO: wait for start signal over channel instead of polling
        poll_interval.tick().await;

        trace!("Poloutboundng server state for holding client...");

        // TODO: shouldn't do this here
        send_keep_alive(writer).await?;

        match server.state() {
            // Still waiting on server start
            State::Starting => {
                trace!(target: "lazymc", "Server not ready, holding client for longer");

                // TODO: timeout
                // // If hold timeout is reached, kick client
                // if since.elapsed().as_secs() >= timeout {
                //     warn!(target: "lazymc", "Held client reached timeout of {}s, disconnecting", timeout);
                //     kick(&config.messages.login_starting, &mut inbound.split().1).await?;
                //     return Ok(());
                // }

                continue;
            }

            // Server started, start relaying and proxy
            State::Started => {
                // TODO: drop client if already disconnected

                // // Relay client to proxy
                // info!(target: "lazymc", "Server ready for held client, relaying to server");
                // service::server::route_proxy_queue(inbound, config, hold_queue);

                info!(target: "lazymc", "Server ready for lobby client, connecting to server");
                return Ok(());
            }

            // Server stopping, this shouldn't happen, kick
            State::Stopping => {
                // TODO: kick message
                // warn!(target: "lazymc", "Server stopping for held client, disconnecting");
                // kick(&config.messages.login_stopping, &mut inbound.split().1).await?;
                break;
            }

            // Server stopped, this shouldn't happen, disconnect
            State::Stopped => {
                error!(target: "lazymc", "Server stopped for held client, disconnecting");
                break;
            }
        }
    }

    Err(())
}

pub async fn connect_to_server(
    real_client: Client,
    config: &Config,
    server: Arc<Server>,
) -> Result<(TcpStream, BytesMut), ()> {
    // Set up connection to server
    // TODO: on connect fail, ping server and redirect to serve_status if offline
    let mut outbound = TcpStream::connect(config.server.address)
        .await
        .map_err(|_| ())?;

    let (mut reader, mut writer) = outbound.split();

    let tmp_client = Client::default();
    tmp_client.set_state(ClientState::Login);

    // TODO: use client version
    let packet = Handshake {
        protocol_version: 755,
        server_addr: config.server.address.ip().to_string(),
        server_port: config.server.address.port(),
        next_state: ClientState::Login.to_id(),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let request = RawPacket::new(proto::HANDSHAKE_PACKET_ID_HANDSHAKE, data).encode()?;
    writer.write_all(&request).await.map_err(|_| ())?;

    // Request login start
    // TODO: use client username
    let packet = LoginStart {
        name: "timvisee".into(),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let request = RawPacket::new(proto::LOGIN_PACKET_ID_LOGIN_START, data).encode()?;
    writer.write_all(&request).await.map_err(|_| ())?;

    // # Wait for server responses

    // Incoming buffer and packet holding queue
    let mut buf = BytesMut::new();
    let mut client_queue = BytesMut::new();
    let mut server_queue = BytesMut::new();

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
        let client_state = tmp_client.state();

        // Hijack login success
        if client_state == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_SUCCESS {
            debug!(target: "LOBBY", "Login success received");

            // TODO: fix reading this packet
            // let login_success =
            //     LoginSuccess::decode(&mut packet.data.as_slice()).map_err(|err| {
            //         dbg!(err);
            //         ()
            //     })?;

            // // TODO: remove debug message
            // debug!(target: "LOBBY", "Login success: {:?}", login_success.username);

            // Switch to play state
            tmp_client.set_state(ClientState::Play);

            debug!(target: "LOBBY", "Server connection ready");

            // TODO: also return buf!
            assert!(
                buf.is_empty(),
                "server incomming buf not empty, will lose data"
            );

            return Ok((outbound, client_queue));
        }

        // // Hijack join game
        // if client_state == ClientState::Play && packet.id == proto::packets::play::CLIENT_JOIN_GAME
        // {
        //     // We must receive join game packet

        //     // TODO: also parse packet!

        //     // TODO: remove debug message
        //     debug!(target: "LOBBY", "Received join packet, will relay to client");

        //     // TODO: send join game packet?
        //     debug!(target: "LOBBY", "client_queue + {} bytes", raw.len());
        //     client_queue.extend(raw);

        //     // TODO: real client and tmp_client must match state

        //     return Ok((outbound, client_queue));
        // }

        // TODO: // Hijack join game
        // TODO: // TODO: client_state == ClientState::Play &&
        // TODO: if packet.id == proto::packets::play::CLIENT_JOIN_GAME {
        // TODO:     let join_game = JoinGame::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

        // TODO:     // TODO: remove debug message
        // TODO:     debug!(target: "LOBBY", "GOT JOIN GAME!");

        // TODO:     continue;
        // TODO: }

        // // Hijack disconnect message
        // // TODO: remove this?
        // if packet.id == proto::packets::play::CLIENT_DISCONNECT {
        //     let disconnect = GameDisconnect::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

        //     debug!(target: "LOBBY", "DISCONNECT REASON: {:?}", disconnect.reason);

        //     continue;
        // }

        // // Grab client state
        // let client_state = client.state();

        // // Hijack login start
        // if client_state == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_START {
        //     // Try to get login username
        //     let login_start = LoginStart::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

        //     // TODO: remove debug message
        //     debug!(target: "LOBBY", "Login {:?}", login_start.name);

        //     // Respond with login success
        //     let packet = LoginSuccess {
        //         // TODO: use correct username here
        //         uuid: Uuid::new_v3(
        //             &Uuid::new_v3(&Uuid::NAMESPACE_OID, b"OfflinePlayer"),
        //             login_start.name.as_bytes(),
        //         ),
        //         username: login_start.name,
        //         // uuid: Uuid::parse_str("35ee313b-d89a-41b8-b25e-d32e8aff0389").unwrap(),
        //         // username: "Username".into(),
        //     };

        //     let mut data = Vec::new();
        //     packet.encode(&mut data).map_err(|_| ())?;

        //     let response = RawPacket::new(proto::LOGIN_PACKET_ID_LOGIN_SUCCESS, data).encode()?;
        //     writer.write_all(&response).await.map_err(|_| ())?;

        //     // Update client state to play
        //     client.set_state(ClientState::Play);

        //     // TODO: remove debug message
        //     debug!(target: "LOBBY", "Sent login success, moving to play state");

        //     // TODO: handle errors here
        //     play_packets(&mut writer).await;

        //     send_keep_alive(&mut writer).await?;

        //     // Wait for server to come online
        //     wait_for_server(config.clone(), server.clone()).await?;

        //     // Connect to server
        //     connect_to_server(client, config, server).await?;

        //     // Keep sending keep alive packets
        //     debug!(target: "LOBBY", "Keep sending keep-alive now...");
        //     send_keep_alive_loop(&mut writer).await?;

        //     debug!(target: "LOBBY", "Done with playing packets, client disconnect?");

        //     return Ok(());
        // }

        // if client_state == ClientState::Play
        //     && packet.id == proto::packets::play::SERVER_CLIENT_SETTINGS
        // {
        //     debug!(target: "LOBBY", "Ignoring client settings packet");
        //     continue;
        // }

        // if client_state == ClientState::Play
        //     && packet.id == proto::packets::play::SERVER_PLUGIN_MESSAGE
        // {
        //     debug!(target: "LOBBY", "Ignoring plugin message packet");
        //     continue;
        // }

        // if client_state == ClientState::Play
        //     && packet.id == proto::packets::play::SERVER_PLAYER_POS_ROT
        // {
        //     debug!(target: "LOBBY", "Ignoring player pos rot packet");
        //     continue;
        // }

        // if client_state == ClientState::Play && packet.id == proto::packets::play::SERVER_PLAYER_POS
        // {
        //     debug!(target: "LOBBY", "Ignoring player pos packet");
        //     continue;
        // }

        // Show unhandled packet warning
        debug!(target: "lazymc", "Received unhandled packet:");
        debug!(target: "lazymc", "- State: {:?}", client_state);
        debug!(target: "lazymc", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    // TODO: we should receive login success from server
    // TODO: we should receive join game packet, relay this to client

    // // Gracefully close connection
    // match writer.shutdown().await {
    //     Ok(_) => {}
    //     Err(err) if err.kind() == io::ErrorKind::NotConnected => {}
    //     Err(_) => return Err(()),
    // }

    // We only reach this on errors
    // TODO: do we actually ever reach this?
    Err(())
}

// TODO: remove unused fields
// TODO: do not drop error here, return Box<dyn Error>
// TODO: add timeout
pub async fn wait_for_server_join_game(
    // client: Client,
    mut outbound: &mut TcpStream,
    // config: Arc<Config>,
    // server: Arc<Server>,
    // queue: BytesMut,
) -> Result<(JoinGame, BytesMut), ()> {
    let (mut reader, mut writer) = outbound.split();

    // TODO: note this assumes the first receiving packet (over queue) is login start
    // TODO: assert client is in login mode!

    // Incoming buffer and packet holding queue
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
        if packet.id == proto::packets::play::CLIENT_JOIN_GAME {
            let join_game = JoinGame::decode(&mut packet.data.as_slice()).map_err(|err| {
                // TODO: remove this debug
                dbg!(err);
                ()
            })?;

            // TODO: remove debug message
            debug!(target: "LOBBY", "GOT JOIN FROM SERVER");

            return Ok((join_game, buf));
        }

        // Show unhandled packet warning
        debug!(target: "lazymc", "Received unhandled packet:");
        // debug!(target: "lazymc", "- State: {:?}", client_state);
        debug!(target: "lazymc", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
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

// TODO: update name and description
/// Route inbound TCP stream to proxy with queued data, spawning a new task.
#[inline]
pub fn route_proxy_queue(
    inbound: TcpStream,
    outbound: TcpStream,
    config: Arc<Config>,
    client_queue: BytesMut,
    server_queue: BytesMut,
) {
    // When server is online, proxy all
    let service = async move {
        proxy::proxy_inbound_outbound_with_queue(inbound, outbound, &client_queue, &server_queue)
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "lazymc", "Failed to proxy: {}", err);
                }

                // TODO: remove after debug
                debug!(target: "LOBBY", "Done with playing packets, client disconnect?");
            })
            .await
    };

    tokio::spawn(service);
}

// TODO: go through this, use proper error messages
pub async fn drain_stream<'a>(reader: &mut ReadHalf<'a>) -> Result<(), ()> {
    // TODO: remove after debug
    debug!(target: "lazymc", "Draining stream...");

    // TODO: use other size?
    let mut drain_buf = [0; 1024];

    loop {
        match reader.try_read(&mut drain_buf) {
            // TODO: stop if read < drain_buf.len() ?
            Ok(read) if read == 0 => return Ok(()),
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
