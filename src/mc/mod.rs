pub mod ban;
#[cfg(feature = "lobby")]
pub mod dimension;
pub mod favicon;
#[cfg(feature = "rcon")]
pub mod rcon;
pub mod server_properties;
#[cfg(feature = "lobby")]
pub mod uuid;
pub mod whitelist;

/// Minecraft ticks per second.
#[allow(unused)]
pub const TICKS_PER_SECOND: u32 = 20;
