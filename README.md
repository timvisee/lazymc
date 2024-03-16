[![Build status on GitLab CI][gitlab-ci-master-badge]][gitlab-ci-link]
[![Project license][license-badge]](LICENSE)

[gitlab-ci-link]: https://gitlab.com/timvisee/lazymc/pipelines
[gitlab-ci-master-badge]: https://gitlab.com/timvisee/lazymc/badges/master/pipeline.svg
[license-badge]: https://img.shields.io/github/license/timvisee/lazymc

# lazymc

`lazymc` puts your Minecraft server to rest when idle, and wakes it up when
players connect.

Some Minecraft servers (especially modded) use an insane amount of resources
when nobody is playing. lazymc helps by stopping your server when idle, until a
player connects again.

lazymc functions as proxy between clients and the server. It handles all
incoming status connections until the server is started and then transparently
relays/proxies the rest. All without them noticing.

https://user-images.githubusercontent.com/856222/141378688-882082be-9efa-4cfe-81cc-5a7ab8b8e86b.mp4


<details><summary>Click to see screenshots</summary>
<p>

![Sleeping server](./res/screenshot/sleeping.png)
![Join sleeping server](./res/screenshot/join.png)
![Starting server](./res/screenshot/starting.png)
![Started server](./res/screenshot/started.png)

</p>
</details>

## Features

- Very efficient, lightweight & low-profile (~3KB RAM)
- Supports Minecraft Java Edition 1.20.3+
- Configure joining client occupation methods:
  - Hold: hold clients when server starts, relay when ready, without them noticing
  - Kick: kick clients when server starts, with a starting message
  - Forward: forward client to another IP when server starts
  - _Lobby: keep client in emulated server with lobby world, teleport to real server when ready ([experimental*](./docs/join-method-lobby.md))_
- Customizable MOTD and login messages
- Automatically manages `server.properties` (host, port and RCON settings)
- Automatically block banned IPs from server within lazymc
- Graceful server sleep/shutdown through RCON or `SIGTERM`
- Real client IP on Minecraft server with `PROXY` header ([usage](./docs/proxy-ip.md))
- Restart server on crash
- Lockout mode

## Requirements

- Linux, macOS or Windows
- Minecraft Java Edition 1.6+
- On Windows: RCON (automatically managed)

Build requirements:

- Rust 1.74 (MSRV)

_Note: You must have access to the system to run the `lazymc` binary. If you're
using a Minecraft shared hosting provider with a custom dashboard, you likely
won't be able to set this up._

## Usage

_Note: these instructions are for Linux & macOS, for Windows look
[here](./docs/usage-windows.md)._

Make sure you meet all [requirements](#requirements).

Download the appropriate binary for your system from the [latest
release][latest-release] page. On macOS you must [compile from
source](#compile-from-source).

Place the binary in your Minecraft server directory, rename it if you like.
Open a terminal, go to the directory, and make sure you can invoke it:

```bash
chmod a+x ./lazymc
./lazymc --help
```

When lazymc is set-up, change into your server directory if you haven't already.
Then set up the [configuration](./res/lazymc.toml) and start it up:

```bash
# Change into your server directory (if you haven't already)
cd server

# Generate lazymc configuration
lazymc config generate

# Edit configuration
# Set the correct server address, directory and start command
nano lazymc.toml

# Start lazymc
lazymc start
```

Please see [extras](./docs/extras.md) for recommendations and additional things
to set up (e.g. how to fix incorrect client IPs and IP banning on your server).

After you've read through the [extras](./docs/extras.md), everything should now
be ready to go! Connect with your Minecraft client to wake your server up!

_Note: If a binary for your system isn't provided, please [compile from
source](#compile-from-source). Installation options are limited at this moment. More will be added
later._

[latest-release]: https://github.com/timvisee/lazymc/releases/latest

## Compile from source

Make sure you meet all [requirements](#requirements).

To compile from source you need Rust, install it through `rustup`: https://rustup.rs/

When Rust is installed, compile and install `lazymc` from this git repository
directly:

```bash
# Compile and install lazymc from source
cargo install -f --git https://github.com/timvisee/lazymc

# Ensure lazymc works
lazymc --help
```

Or clone the repository and build it yourself:

```bash
# Clone repository
git clone https://github.com/timvisee/lazymc
cd lazymc

# Compile
cargo build --release

# Run lazymc
./target/release/lazymc --help
```

## Third-party usage & implementations

A list of third-party implementations, projects using lazymc, that you might
find useful:

- Docker: [crbanman/papermc-lazymc](https://hub.docker.com/r/crbanman/papermc-lazymc) _(PaperMC with lazymc in Docker)_

## License

This project is released under the GNU GPL-3.0 license.
Check out the [LICENSE](LICENSE) file for more information.
