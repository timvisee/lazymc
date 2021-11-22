# Proxy IP

lazymc acts as a proxy most of the time. Because of this the Minecraft server
will think all clients connect from the same IP, being the IP lazymc proxies
from.

This breaks IP banning (`/ban-ip`, amongst other IP related things). This may be
a problematic issue for your server.

Luckily, this can be fixed with the [proxy header](#proxy-header). lazymc has
support for this, and can be used with a companion plugin on your server.

## Proxy header

The `PROXY` header may be used to notify the Minecraft server of the real client
IP.

When a new connection is opened to the Minecraft server, the Minecraft server
will read the `PROXY` header with client-IP information. Once read, it will set
the correct client IP internally and will resume communicating with the client
normally.

To enable this with lazymc you must do two things:
- [Modify the lazymc configuration](#configuration)
- [Install a companion plugin](#server-plugin)

## Configuration

To use the `PROXY` header with your Minecraft server, set `server.send_proxy_v2`
to `true`.

[`lazymc.toml`](../res/lazymc.toml):

```toml
# -- snip --

[server]
send_proxy_v2 = true

# -- snip --
```

Other related properties, you probably won't need to touch, include:

- `server.send_proxy_v2`: set to `true` to enable `PROXY` header for Minecraft server
- `join.forward.send_proxy_v2`: set to `true` to enable `PROXY` header forwarded server, if `forward` join method is used
- `rcon.send_proxy_v2`: set to `true` to enable `PROXY` header for RCON connections for Minecraft server

## Server plugin

Install one of these plugins as companion on your server to enable support for
the `PROXY` header. This requires Minecraft server software supporting plugins,
the vanilla Minecraft server does not support this.

If lazymc connects to a Spigot compatible server, use any of:

- https://github.com/riku6460/SpigotProxy ([JAR](https://github.com/riku6460/SpigotProxy/releases/latest))
- https://github.com/timvisee/spigot-proxy

If lazymc connects to a BungeeCord server, use any of:

- https://github.com/MinelinkNetwork/BungeeProxy

## Warning: connection failures

Use of the `PROXY` header must be enabled or disabled on both lazymc and your
Minecraft server using a companion plugin.

If either of the two is missing or misconfigured, it will result in connection
failures due to a missing or unrecognized header.

## Warning: fake IP

When enabling the `PROXY` header on your Minecraft server, malicious parties may
send this header to fake their real IP.

To solve this, make sure the Minecraft server is only publicly reachable through
lazymc. This can be done by setting the Minecraft server IP to a local address
only, or by setting up firewall rules.
