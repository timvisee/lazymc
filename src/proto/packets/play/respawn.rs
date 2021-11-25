use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

use super::join_game::JoinGameData;
use crate::mc::dimension;
use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

/// Send respawn packet to client to jump from lobby into now loaded server.
///
/// The required details will be fetched from the `join_game` packet as provided by the server.
pub async fn lobby_send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    data: JoinGameData,
) -> Result<(), ()> {
    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => {
            packet::write_packet(
                v1_16_3::game::Respawn {
                    dimension: data.dimension.unwrap_or_else(|| {
                        dimension::lobby_dimension(
                            &data
                                .dimension_codec
                                .unwrap_or_else(dimension::default_dimension_codec),
                        )
                    }),
                    world_name: data
                        .world_name
                        .unwrap_or_else(|| "minecraft:overworld".into()),
                    hashed_seed: data.hashed_seed.unwrap_or(0),
                    game_mode: data.game_mode.unwrap_or(0),
                    previous_game_mode: data.previous_game_mode.unwrap_or(-1i8 as u8),
                    is_debug: data.is_debug.unwrap_or(false),
                    is_flat: data.is_flat.unwrap_or(false),
                    copy_metadata: false,
                },
                client,
                writer,
            )
            .await
        }
        _ => {
            packet::write_packet(
                v1_17::game::Respawn {
                    dimension: data.dimension.unwrap_or_else(|| {
                        dimension::lobby_dimension(
                            &data
                                .dimension_codec
                                .unwrap_or_else(dimension::default_dimension_codec),
                        )
                    }),
                    world_name: data
                        .world_name
                        .unwrap_or_else(|| "minecraft:overworld".into()),
                    hashed_seed: data.hashed_seed.unwrap_or(0),
                    game_mode: data.game_mode.unwrap_or(0),
                    previous_game_mode: data.previous_game_mode.unwrap_or(-1i8 as u8),
                    is_debug: data.is_debug.unwrap_or(false),
                    is_flat: data.is_flat.unwrap_or(false),
                    copy_metadata: false,
                },
                client,
                writer,
            )
            .await
        }
    }
}
