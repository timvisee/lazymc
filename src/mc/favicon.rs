/// Get default server status favicon.
pub fn default_favicon() -> String {
    encode_favicon(include_bytes!("../../res/unknown_server_optimized.png"))
}

/// Encode favicon bytes to a string Minecraft can read.
///
/// This assumes the favicon data to be a valid PNG image.
pub fn encode_favicon(data: &[u8]) -> String {
    format!("{}{}", "data:image/png;base64,", base64::encode(data))
}
