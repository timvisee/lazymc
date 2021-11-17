pub mod ban;
#[cfg(feature = "rcon")]
pub mod rcon;
pub mod server_properties;

/// Minecraft ticks per second.
#[allow(unused)]
pub const TICKS_PER_SECOND: u32 = 20;
