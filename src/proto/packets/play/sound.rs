use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

/// Play a sound effect at world origin.
pub async fn send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    sound_name: &str,
) -> Result<(), ()> {
    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => {
            packet::write_packet(
                v1_16_3::game::NamedSoundEffect {
                    sound_name: sound_name.into(),
                    sound_category: 0,
                    effect_pos_x: 0,
                    effect_pos_y: 0,
                    effect_pos_z: 0,
                    volume: 1.0,
                    pitch: 1.0,
                },
                client,
                writer,
            )
            .await
        }
        _ => {
            packet::write_packet(
                v1_17::game::NamedSoundEffect {
                    sound_name: sound_name.into(),
                    sound_category: 0,
                    effect_pos_x: 0,
                    effect_pos_y: 0,
                    effect_pos_z: 0,
                    volume: 1.0,
                    pitch: 1.0,
                },
                client,
                writer,
            )
            .await
        }
    }
}
