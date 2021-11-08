/// Command to start server.
pub const SERVER_CMD: &str = "/home/timvisee/git/lazymc/mcserver/start";

/// Public address for users to connect to.
pub const ADDRESS_PUBLIC: &str = "127.0.0.1:9090";

/// Minecraft server address to proxy to.
pub const ADDRESS_PROXY: &str = "127.0.0.1:9091";

/// Server description shown when server is starting.
pub const LABEL_SERVER_SLEEPING: &str = "☠ Server is sleeping\n§2☻ Join to start it up";

/// Server description shown when server is starting.
pub const LABEL_SERVER_STARTING: &str = "§2☻ Server is starting...\n§7⌛ Please wait...";

/// Kick message shown when user tries to connect to starting server.
pub const LABEL_SERVER_STARTING_MESSAGE: &str =
    "Server is starting... §c♥§r\n\nThis may take some time.\n\nPlease try to reconnect in a minute.";

/// Idle server sleeping delay in seconds.
pub const SLEEP_DELAY: u64 = 10;
