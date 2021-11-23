use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

/// Send lobby time update to client.
///
/// Sets world time to 0.
///
/// Required once for keep-alive packets.
pub async fn send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => {
            packet::write_packet(
                v1_16_3::game::TimeUpdate {
                    world_age: 0,
                    time_of_day: 0,
                },
                client,
                writer,
            )
            .await
        }
        _ => {
            packet::write_packet(
                v1_17::game::TimeUpdate {
                    world_age: 0,
                    time_of_day: 0,
                },
                client,
                writer,
            )
            .await
        }
    }
}
