use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::game::GameDisconnect;
use minecraft_protocol::version::v1_14_4::login::LoginDisconnect;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientState};
use crate::proto::packet::RawPacket;
use crate::proto::packets;

/// Kick client with a message.
///
/// Should close connection afterwards.
pub async fn kick(client: &Client, msg: &str, writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    match client.state() {
        ClientState::Login => login_kick(client, msg, writer).await,
        ClientState::Play => play_kick(client, msg, writer).await,
        _ => Err(()),
    }
}

/// Kick client with a message in login state.
///
/// Should close connection afterwards.
async fn login_kick(client: &Client, msg: &str, writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let packet = LoginDisconnect {
        reason: Message::new(Payload::text(msg)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(packets::login::CLIENT_DISCONNECT, data).encode(client)?;
    writer.write_all(&response).await.map_err(|_| ())
}

/// Kick client with a message in play state.
///
/// Should close connection afterwards.
async fn play_kick(client: &Client, msg: &str, writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    let packet = GameDisconnect {
        reason: Message::new(Payload::text(msg)),
    };

    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(packets::play::CLIENT_DISCONNECT, data).encode(client)?;
    writer.write_all(&response).await.map_err(|_| ())
}
