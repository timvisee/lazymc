use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::error::DecodeError;
use minecraft_protocol::version::{v1_16_3, v1_17};
use nbt::CompoundTag;
#[cfg(feature = "lobby")]
use tokio::net::tcp::WriteHalf;

#[cfg(feature = "lobby")]
use crate::mc::dimension;
#[cfg(feature = "lobby")]
use crate::proto::client::Client;
use crate::proto::client::ClientInfo;
#[cfg(feature = "lobby")]
use crate::proto::packet;
use crate::proto::packet::RawPacket;
#[cfg(feature = "lobby")]
use crate::server::Server;

/// Data extracted from `JoinGame` packet.
#[derive(Debug, Clone)]
pub struct JoinGameData {
    pub dimension: Option<CompoundTag>,
    pub dimension_codec: Option<CompoundTag>,
    pub world_name: Option<String>,
    pub hashed_seed: Option<i64>,
    pub game_mode: Option<u8>,
    pub previous_game_mode: Option<u8>,
    pub is_debug: Option<bool>,
    pub is_flat: Option<bool>,
}

impl JoinGameData {
    /// Extract join game data from given packet.
    pub fn from_packet(client_info: &ClientInfo, packet: RawPacket) -> Result<Self, DecodeError> {
        match client_info.protocol() {
            Some(p) if p <= v1_16_3::PROTOCOL => {
                Ok(v1_16_3::game::JoinGame::decode(&mut packet.data.as_slice())?.into())
            }
            _ => Ok(v1_17::game::JoinGame::decode(&mut packet.data.as_slice())?.into()),
        }
    }
}

impl From<v1_16_3::game::JoinGame> for JoinGameData {
    fn from(join_game: v1_16_3::game::JoinGame) -> Self {
        Self {
            dimension: Some(join_game.dimension),
            dimension_codec: Some(join_game.dimension_codec),
            world_name: Some(join_game.world_name),
            hashed_seed: Some(join_game.hashed_seed),
            game_mode: Some(join_game.game_mode),
            previous_game_mode: Some(join_game.previous_game_mode),
            is_debug: Some(join_game.is_debug),
            is_flat: Some(join_game.is_flat),
        }
    }
}

impl From<v1_17::game::JoinGame> for JoinGameData {
    fn from(join_game: v1_17::game::JoinGame) -> Self {
        Self {
            dimension: Some(join_game.dimension),
            dimension_codec: Some(join_game.dimension_codec),
            world_name: Some(join_game.world_name),
            hashed_seed: Some(join_game.hashed_seed),
            game_mode: Some(join_game.game_mode),
            previous_game_mode: Some(join_game.previous_game_mode),
            is_debug: Some(join_game.is_debug),
            is_flat: Some(join_game.is_flat),
        }
    }
}

/// Check whether the packet ID matches.
pub fn is_packet(client_info: &ClientInfo, packet_id: u8) -> bool {
    match client_info.protocol() {
        Some(p) if p <= v1_16_3::PROTOCOL => packet_id == v1_16_3::game::JoinGame::PACKET_ID,
        _ => packet_id == v1_17::game::JoinGame::PACKET_ID,
    }
}

/// Send initial join game packet to client for lobby.
#[cfg(feature = "lobby")]
pub async fn lobby_send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    server: &Server,
) -> Result<(), ()> {
    // Get dimension codec and build lobby dimension
    let dimension_codec: CompoundTag =
        if let Some(ref join_game) = server.probed_join_game.lock().await.as_ref() {
            join_game
                .dimension_codec
                .clone()
                .unwrap_or_else(|| dimension::default_dimension_codec())
        } else {
            dimension::default_dimension_codec()
        };
    let dimension: CompoundTag = dimension::lobby_dimension(&dimension_codec);

    let status = server.status().await;

    match client_info.protocol() {
        Some(p) if p <= v1_16_3::PROTOCOL => {
            packet::write_packet(
                v1_16_3::game::JoinGame {
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
                    dimension_codec,
                    dimension,
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
                },
                client,
                writer,
            )
            .await
        }
        _ => {
            packet::write_packet(
                v1_17::game::JoinGame {
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
                    dimension_codec,
                    dimension,
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
                },
                client,
                writer,
            )
            .await
        }
    }
}
