// TODO: remove this before feature release!
#![allow(unused)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::data::server_status::*;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::game::{GameMode, MessagePosition};
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::{LoginDisconnect, LoginStart, LoginSuccess};
use minecraft_protocol::version::v1_14_4::status::StatusResponse;
use minecraft_protocol::version::v1_17_1::game::{
    ChunkData, ClientBoundChatMessage, ClientBoundKeepAlive, JoinGame, PlayerPositionAndLook,
    SetTitleSubtitle, SetTitleText, SetTitleTimes, SpawnPosition, TimeUpdate,
};
use nbt::CompoundTag;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::tcp::WriteHalf;
use tokio::net::TcpStream;
use tokio::time;
use uuid::Uuid;

use crate::config::*;
use crate::proto::{self, Client, ClientState, RawPacket};
use crate::server::{self, Server, State};
use crate::service;

// TODO: remove this before releasing feature
pub const USE_LOBBY: bool = true;
pub const DONT_START_SERVER: bool = true;
const STARTING_BANNER: &str = "§2 Server is starting...";
const STARTING_BANNER_SUB: &str = "§7⌛ Please wait...";

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
    let mut hold_queue = BytesMut::new();

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

            debug!(target: "LOBBY", "Done with playing packets, client disconnect?");

            break;
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

    // // Keep sending keep alive packets
    send_keep_alive_loop(writer).await?;

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
        entity_id: 0x6d,
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
        world_name: "minecraft:overworld".into(),
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

        send_title(writer).await?;
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
