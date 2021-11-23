use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

/// Minecraft channel to set brand.
const CHANNEL: &str = "minecraft:brand";

/// Server brand to send to client in lobby world.
///
/// Shown in F3 menu. Updated once client is relayed to real server.
const SERVER_BRAND: &[u8] = b"lazymc";

/// Send lobby brand to client.
pub async fn send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => {
            packet::write_packet(
                v1_16_3::game::ClientBoundPluginMessage {
                    channel: CHANNEL.into(),
                    data: SERVER_BRAND.into(),
                },
                client,
                writer,
            )
            .await
        }
        _ => {
            packet::write_packet(
                v1_17::game::ClientBoundPluginMessage {
                    channel: CHANNEL.into(),
                    data: SERVER_BRAND.into(),
                },
                client,
                writer,
            )
            .await
        }
    }
}
