use base64::Engine;

use crate::proto::client::ClientInfo;

/// Protocol version since when favicons are supported.
const FAVICON_PROTOCOL_VERSION: u32 = 4;

/// Get default server status favicon.
pub fn default_favicon() -> String {
    encode_favicon(include_bytes!("../../res/unknown_server_optimized.png"))
}

/// Encode favicon bytes to a string Minecraft can read.
///
/// This assumes the favicon data to be a valid PNG image.
pub fn encode_favicon(data: &[u8]) -> String {
    format!(
        "{}{}",
        "data:image/png;base64,",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

/// Check whether the status response favicon is supported based on the given client info.
///
/// Defaults to `true` if unsure.
pub fn supports_favicon(client_info: &ClientInfo) -> bool {
    client_info
        .protocol
        .map(|p| p >= FAVICON_PROTOCOL_VERSION)
        .unwrap_or(true)
}
