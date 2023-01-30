# Protocol version

The Minecraft protocol uses a version number to distinguish between different
protocol versions. Each new Minecraft version having a change in its protocol
gets a new protocol version.

## List of versions

- https://wiki.vg/Protocol_version_numbers#Versions_after_the_Netty_rewrite

## Configuration

In lazymc you may configure what protocol version to use:

[`lazymc.toml`](../res/lazymc.toml):

```bash
# -- snip --

[public]
# Server version & protocol hint.
# Sent to clients until actual server version is known.
# See: https://git.io/J1Fvx
version = "1.19.3"
protocol = 761

# -- snip --
```

It is highly recommended to set these to match that of your server version to
allow the best compatibility with clients.

- Set `public.protocol` to the number matching that of your server version
  (see [this](#list-of-versions) list)
- Set `public.version` to any string you like. Shows up in read in clients that
  have an incompatibel protocol version number

These are used as hint. lazymc will automatically use the protocol version of
your Minecraft server once it has started at least once.
