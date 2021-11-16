use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::version::v1_14_4::game::GameDisconnect;
use minecraft_protocol::version::v1_14_4::login::LoginDisconnect;
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientState};
use crate::proto::packet;

/// Kick client with a message.
///
/// Should close connection afterwards.
pub async fn kick(client: &Client, msg: &str, writer: &mut WriteHalf<'_>) -> Result<(), ()> {
    match client.state() {
        ClientState::Login => {
            packet::write_packet(
                LoginDisconnect {
                    reason: Message::new(Payload::text(msg)),
                },
                client,
                writer,
            )
            .await
        }
        ClientState::Play => {
            packet::write_packet(
                GameDisconnect {
                    reason: Message::new(Payload::text(msg)),
                },
                client,
                writer,
            )
            .await
        }
        _ => Err(()),
    }
}
