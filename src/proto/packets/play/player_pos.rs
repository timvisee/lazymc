use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

/// Move player to world origin.
pub async fn send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => {
            packet::write_packet(
                v1_16_3::game::PlayerPositionAndLook {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                    pitch: 90.0,
                    flags: 0b00000000,
                    teleport_id: 0,
                },
                client,
                writer,
            )
            .await
        }
        _ => {
            packet::write_packet(
                v1_17::game::PlayerPositionAndLook {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                    pitch: 90.0,
                    flags: 0b00000000,
                    teleport_id: 0,
                    dismount_vehicle: true,
                },
                client,
                writer,
            )
            .await
        }
    }
}
