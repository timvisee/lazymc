# Join method: transfer

The transfer join method holds back a joining client in the configuration state
while the server starts, then tells the client to reconnect once the server is
ready. The client reconnects on its own, so the player joins with a single
connection attempt and never sees a kick message.

This is the recommended method for clients on Minecraft 1.20.5 and above.

## Why not hold?

The `hold` method keeps the client in the login state without sending anything.
Minecraft clients time out after 30 seconds there, so servers that take longer to
start - modded servers in particular - always fall through to `kick`, and the
player has to reconnect by hand.

In the configuration state, which was added in Minecraft 1.20.2, the server may
send keep alive packets. A client that receives them waits indefinitely, which
removes the timeout problem entirely.

## Why not lobby?

The `lobby` method emulates a fake server, which means it has to reproduce the
registry data the real server would send. That is version specific, and it does
not work with modded servers at all.

The transfer method sidesteps this. It never pretends to be the real server: it
only parks the client until the real server is up, then hands it over. The client
then performs a full handshake with the real server, including any mod handshake,
so Forge, NeoForge and modpacks keep working without lazymc knowing anything
about them.

## Requirements

- The client must use Minecraft 1.20.5 or above (protocol 766+), because the
  transfer packet does not exist before that. Older clients are passed on to the
  next configured join method.
- The server must accept transfers. Set `accepts-transfers=true` in
  `server.properties`, otherwise the client is refused with
  `Server does not accept transfers`.

## Usage

```toml
[join]
methods = ["transfer", "kick"]

[join.transfer]
# Maximum time to hold the client while the server starts.
timeout = 600

# Interval between keep alive packets.
keep_alive_interval = 10

# Where the client should reconnect to. Defaults to the public address lazymc
# listens on, which is usually what you want.
#address = "example.com:25565"
```

Keeping `kick` after `transfer` is recommended, so clients that are too old still
get a readable message.

## Notes

The address the client is sent to must be reachable by the client, not by lazymc.
The default is the public address lazymc listens on, so the client reconnects
through lazymc and is proxied to the server as usual.

While held, the client shows its regular connecting screen. Use the `[motd]`
section to tell players the server is sleeping before they connect.
