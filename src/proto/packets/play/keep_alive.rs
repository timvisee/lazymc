use std::sync::atomic::{AtomicU64, Ordering};

use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

/// Auto incrementing ID source for keep alive packets.
static KEEP_ALIVE_ID: AtomicU64 = AtomicU64::new(0);

/// Send keep alive packet to client.
///
/// Required periodically in play mode to prevent client timeout.
pub async fn send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    // Keep sending new IDs
    let id = KEEP_ALIVE_ID.fetch_add(1, Ordering::Relaxed);

    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => {
            packet::write_packet(v1_16_3::game::ClientBoundKeepAlive { id }, client, writer).await
        }
        _ => packet::write_packet(v1_17::game::ClientBoundKeepAlive { id }, client, writer).await,
    }
}
