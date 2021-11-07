/// Command to start server.
pub(crate) const SERVER_CMD: &str = "/home/timvisee/git/lazymc/mcserver/start";

/// Public address for users to connect to.
pub(crate) const ADDRESS_PUBLIC: &str = "127.0.0.1:9090";

/// Minecraft server address to proxy to.
pub(crate) const ADDRESS_PROXY: &str = "127.0.0.1:9091";

/// Server description shown when server is starting.
pub(crate) const LABEL_SERVER_SLEEPING: &str = "☠ Server is sleeping\n§2☻ Join to start it up";

/// Server description shown when server is starting.
pub(crate) const LABEL_SERVER_STARTING: &str = "§2☻ Server is starting...\n§7⌛ Please wait...";

/// Kick message shown when user tries to connect to starting server.
pub(crate) const LABEL_SERVER_STARTING_MESSAGE: &str =
    "Server is starting... §c♥§r\n\nThis may take some time.\n\nPlease try to reconnect in a minute.";
