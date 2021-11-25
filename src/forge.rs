#[cfg(feature = "lobby")]
use std::sync::Arc;
#[cfg(feature = "lobby")]
use std::time::Duration;

#[cfg(feature = "lobby")]
use bytes::BytesMut;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::forge_v1_13::login::{Acknowledgement, LoginWrapper, ModList};
use minecraft_protocol::version::v1_14_4::login::{LoginPluginRequest, LoginPluginResponse};
use minecraft_protocol::version::PacketId;
#[cfg(feature = "lobby")]
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::WriteHalf;
#[cfg(feature = "lobby")]
use tokio::net::TcpStream;
#[cfg(feature = "lobby")]
use tokio::time;

use crate::forge;
use crate::proto::client::Client;
#[cfg(feature = "lobby")]
use crate::proto::client::ClientState;
use crate::proto::packet;
use crate::proto::packet::RawPacket;
#[cfg(feature = "lobby")]
use crate::proto::packets;
#[cfg(feature = "lobby")]
use crate::server::Server;

/// Forge status magic.
pub const STATUS_MAGIC: &str = "\0FML2\0";

/// Forge plugin wrapper login plugin request channel.
pub const CHANNEL_LOGIN_WRAPPER: &str = "fml:loginwrapper";

/// Forge handshake channel.
pub const CHANNEL_HANDSHAKE: &str = "fml:handshake";

/// Timeout for draining Forge plugin responses from client.
#[cfg(feature = "lobby")]
const CLIENT_DRAIN_FORGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Respond with Forge login wrapper packet.
pub async fn respond_forge_login_packet(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    message_id: i32,
    forge_channel: String,
    forge_packet: impl PacketId + Encoder,
) -> Result<(), ()> {
    // Encode Forge packet to data
    let mut forge_data = Vec::new();
    forge_packet.encode(&mut forge_data).map_err(|_| ())?;

    // Encode Forge payload
    let forge_payload =
        RawPacket::new(forge_packet.packet_id(), forge_data).encode_without_len(client)?;

    // Wrap Forge payload in login wrapper
    let mut payload = Vec::new();
    let packet = LoginWrapper {
        channel: forge_channel,
        packet: forge_payload,
    };
    packet.encode(&mut payload).map_err(|_| ())?;

    // Write login plugin request with forge payload
    packet::write_packet(
        LoginPluginResponse {
            message_id,
            successful: true,
            data: payload,
        },
        client,
        writer,
    )
    .await
}

/// Respond to a Forge login plugin request.
pub async fn respond_login_plugin_request(
    client: &Client,
    packet: LoginPluginRequest,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    // Decode Forge login wrapper packet
    let (message_id, login_wrapper, packet) =
        forge::decode_forge_login_packet(client, packet).await?;

    // Determine whether we received the mod list
    let is_unknown_header = login_wrapper.channel != forge::CHANNEL_HANDSHAKE;
    let is_mod_list = !is_unknown_header && packet.id == ModList::PACKET_ID;

    // If not the mod list, just acknowledge
    if !is_mod_list {
        trace!(target: "lazymc::forge", "Acknowledging login plugin request");
        forge::respond_forge_login_packet(
            client,
            writer,
            message_id,
            login_wrapper.channel,
            Acknowledgement {},
        )
        .await
        .map_err(|_| {
            error!(target: "lazymc::forge", "Failed to send Forge login plugin request acknowledgement");
        })?;
        return Ok(());
    }

    trace!(target: "lazymc::forge", "Sending mod list reply to server with same contents");

    // Parse mod list, transform into reply
    let mod_list = ModList::decode(&mut packet.data.as_slice()).map_err(|err| {
        error!(target: "lazymc::forge", "Failed to decode Forge mod list: {:?}", err);
    })?;
    let mod_list_reply = mod_list.into_reply();

    // We got mod list, respond with reply
    forge::respond_forge_login_packet(
        client,
        writer,
        message_id,
        login_wrapper.channel,
        mod_list_reply,
    )
    .await
    .map_err(|_| {
        error!(target: "lazymc::forge", "Failed to send Forge login plugin mod list reply");
    })?;

    Ok(())
}

/// Decode a Forge login wrapper packet from login plugin request.
///
/// Returns (`message_id`, `login_wrapper`, `packet`).
pub async fn decode_forge_login_packet(
    client: &Client,
    plugin_request: LoginPluginRequest,
) -> Result<(i32, LoginWrapper, RawPacket), ()> {
    // Validate channel
    assert_eq!(plugin_request.channel, CHANNEL_LOGIN_WRAPPER);

    // Decode login wrapped packet
    let login_wrapper =
        LoginWrapper::decode(&mut plugin_request.data.as_slice()).map_err(|err| {
            error!(target: "lazymc::forge", "Failed to decode Forge LoginWrapper packet: {:?}", err);
        })?;

    // Parse packet
    let packet = RawPacket::decode_without_len(client, &login_wrapper.packet).map_err(|err| {
        error!(target: "lazymc::forge", "Failed to decode Forge LoginWrapper packet contents: {:?}", err);
    })?;

    Ok((plugin_request.message_id, login_wrapper, packet))
}

/// Replay the Forge login payload for a client.
#[cfg(feature = "lobby")]
pub async fn replay_login_payload(
    client: &Client,
    inbound: &mut TcpStream,
    server: Arc<Server>,
    inbound_buf: &mut BytesMut,
) -> Result<(), ()> {
    debug!(target: "lazymc::lobby", "Replaying Forge login procedure for lobby client...");

    // Replay each Forge packet
    for packet in server.forge_payload.read().await.as_slice() {
        inbound.write_all(packet).await.map_err(|err| {
            error!(target: "lazymc::lobby", "Failed to send Forge join payload to lobby client, will likely cause issues: {}", err);
        })?;
    }

    // Drain all responses
    let count = server.forge_payload.read().await.len();
    drain_forge_responses(client, inbound, inbound_buf, count).await?;

    trace!(target: "lazymc::lobby", "Forge join payload replayed");

    Ok(())
}

/// Drain Forge login plugin response packets from stream.
#[cfg(feature = "lobby")]
async fn drain_forge_responses(
    client: &Client,
    inbound: &mut TcpStream,
    buf: &mut BytesMut,
    mut count: usize,
) -> Result<(), ()> {
    let (mut reader, mut _writer) = inbound.split();

    loop {
        // We're done if count is zero
        if count == 0 {
            trace!(target: "lazymc::forge", "Drained all plugin responses from client");
            return Ok(());
        }

        // Read packet from stream with timeout
        let read_packet_task = packet::read_packet(client, buf, &mut reader);
        let timeout = time::timeout(CLIENT_DRAIN_FORGE_TIMEOUT, read_packet_task).await;
        let read_packet_task = match timeout {
            Ok(result) => result,
            Err(_) => {
                error!(target: "lazymc::forge", "Expected more plugin responses from client, but didn't receive anything in a while, may be problematic");
                return Ok(());
            }
        };

        // Read packet from stream
        let (packet, _raw) = match read_packet_task {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "lazymc::forge", "Closing connection, error occurred");
                break;
            }
        };

        // Grab client state
        let client_state = client.state();

        // Catch login plugin resposne
        if client_state == ClientState::Login
            && packet.id == packets::login::SERVER_LOGIN_PLUGIN_RESPONSE
        {
            trace!(target: "lazymc::forge", "Voiding plugin response from client");
            count -= 1;
            continue;
        }

        // TODO: instantly return on this packet?
        // // Hijack login success
        // if client_state == ClientState::Login && packet.id == packets::login::CLIENT_LOGIN_SUCCESS {
        //     trace!(target: "lazymc::forge", "Got login success from server connection, change to play mode");

        //     // Switch to play state
        //     tmp_client.set_state(ClientState::Play);

        //     return Ok(forge_payload);
        // }

        // Show unhandled packet warning
        debug!(target: "lazymc::forge", "Got unhandled packet from server in record_forge_response:");
        debug!(target: "lazymc::forge", "- State: {:?}", client_state);
        debug!(target: "lazymc::forge", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    Err(())
}
