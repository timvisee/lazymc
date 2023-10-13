use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

#[cfg(feature = "lobby")]
use crate::lobby::KEEP_ALIVE_INTERVAL;
use crate::mc;
use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

#[cfg(feature = "lobby")]
const DISPLAY_TIME: i32 = KEEP_ALIVE_INTERVAL.as_secs() as i32 * mc::TICKS_PER_SECOND as i32 * 2;
#[cfg(not(feature = "lobby"))]
const DISPLAY_TIME: i32 = 10 * mc::TICKS_PER_SECOND as i32 * 2;

/// Send lobby title packets to client.
///
/// This will show the given text for two keep-alive periods. Use a newline for the subtitle.
///
/// If an empty string is given, the title times will be reset to default.
pub async fn send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    text: &str,
) -> Result<(), ()> {
    // Grab title and subtitle bits
    let title = text.lines().next().unwrap_or("");
    let subtitle = text.lines().skip(1).collect::<Vec<_>>().join("\n");

    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => send_v1_16_3(client, writer, title, &subtitle).await,
        _ => send_v1_17(client, writer, title, &subtitle).await,
    }
}

async fn send_v1_16_3(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    title: &str,
    subtitle: &str,
) -> Result<(), ()> {
    use v1_16_3::game::{Title, TitleAction};

    // Set title
    packet::write_packet(
        Title {
            action: TitleAction::SetTitle {
                text: Message::new(Payload::text(title)),
            },
        },
        client,
        writer,
    )
    .await?;

    // Set subtitle
    packet::write_packet(
        Title {
            action: TitleAction::SetSubtitle {
                text: Message::new(Payload::text(subtitle)),
            },
        },
        client,
        writer,
    )
    .await?;

    // Set title times
    packet::write_packet(
        Title {
            action: if title.is_empty() && subtitle.is_empty() {
                // Defaults: https://minecraft.wiki/w/Commands/title#Detail
                TitleAction::SetTimesAndDisplay {
                    fade_in: 10,
                    stay: 70,
                    fade_out: 20,
                }
            } else {
                TitleAction::SetTimesAndDisplay {
                    fade_in: 0,
                    stay: DISPLAY_TIME,
                    fade_out: 0,
                }
            },
        },
        client,
        writer,
    )
    .await
}

async fn send_v1_17(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    title: &str,
    subtitle: &str,
) -> Result<(), ()> {
    use v1_17::game::{SetTitleSubtitle, SetTitleText, SetTitleTimes};

    // Set title
    packet::write_packet(
        SetTitleText {
            text: Message::new(Payload::text(title)),
        },
        client,
        writer,
    )
    .await?;

    // Set subtitle
    packet::write_packet(
        SetTitleSubtitle {
            text: Message::new(Payload::text(subtitle)),
        },
        client,
        writer,
    )
    .await?;

    // Set title times
    packet::write_packet(
        if title.is_empty() && subtitle.is_empty() {
            // Defaults: https://minecraft.wiki/w/Commands/title#Detail
            SetTitleTimes {
                fade_in: 10,
                stay: 70,
                fade_out: 20,
            }
        } else {
            SetTitleTimes {
                fade_in: 0,
                stay: DISPLAY_TIME,
                fade_out: 0,
            }
        },
        client,
        writer,
    )
    .await
}
