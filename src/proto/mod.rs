pub mod action;
pub mod client;
pub mod packet;
pub mod packets;

/// Default minecraft protocol version name.
///
/// Just something to default to when real server version isn't known or when no hint is specified
/// in the configuration.
///
/// Should be kept up-to-date with latest supported Minecraft version by lazymc.
pub const PROTO_DEFAULT_VERSION: &str = "1.20.3";

/// Default minecraft protocol version.
///
/// Just something to default to when real server version isn't known or when no hint is specified
/// in the configuration.
///
/// Should be kept up-to-date with latest supported Minecraft version by lazymc.
pub const PROTO_DEFAULT_PROTOCOL: u32 = 765;

/// Compression threshold to use.
// TODO: read this from server.properties instead
pub const COMPRESSION_THRESHOLD: i32 = 256;

/// Default buffer size when reading packets.
pub(super) const BUF_SIZE: usize = 8 * 1024;
