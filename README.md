# lazymc

`lazymc` puts your Minecraft server to rest when idle, and wakes it up when
players connect.

Some Minecraft servers (especially modded) use an insane amount of resources
when nobody is playing. lazymc helps by stopping your server when idle, until a
player connects again.

lazymc functions as proxy between clients and the server. It handles all
incoming status connections until the server is started and then transparently
relays/proxies the rest. All without them noticing.

_Note: this is a prototype and may be incomplete._

https://user-images.githubusercontent.com/856222/140804726-ba1a8e59-85d9-413b-8229-03be84b55d51.mp4

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
- Supports Minecraft Java Edition 1.6+, supports modded (e.g. Forge, FTB)
- Transparent join: hold clients when server starts, relay when ready, without them noticing
- Customizable MOTD and login messages
- Automatically manages `server.properties` (host, port and RCON settings)
- Graceful server sleep/shutdown through RCON (with `SIGTERM` fallback on Linux/Unix)
- Restart server on crash

## Requirements

- Linux, macOS or Windows
- Minecraft Java Edition 1.6+
- On Windows: RCON (automatically managed)

_Note: You must have access to the system to run the `lazymc` binary. If you're
using a Minecraft shared hosting provider with a custom dashboard, you likely
won't be able to set this up._

## Usage

Make sure you meet all [requirements](#requirements).

_Note: Installation options are limited at this moment. Ready-to-go binaries
will be published later. For now we compile and install from source._

To compile and install you need Rust, install it through `rustup`: https://rustup.rs/

When Rust is installed, compile and install `lazymc` from this git repository:

```bash
# Compile and install lazymc from source
cargo install -f --git https://github.com/timvisee/lazymc

# Ensure lazymc works
lazymc --help
```

When `lazymc` is available, change into your server directory. Then set up the
[configuration](./res/lazymc.toml) and start it up:

```bash
# Change into your server directory
cd server

# Generate lazymc configuration
lazymc config generate

# Edit configuration
# Set the correct server address, directory and start command
nano lazymc.toml

# Start lazymc
lazymc start
```

Everything should now be running. Connect with your Minecraft client to wake
your server up!

## License

This project is released under the GNU GPL-3.0 license.
Check out the [LICENSE](LICENSE) file for more information.
