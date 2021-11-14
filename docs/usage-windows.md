## Usage on Windows

Make sure you meet all [requirements](../README.md#requirements).

Download the `lazymc-*-windows.exe` Windows executable for your system from the
[latest release][latest-release] page.

Place the binary in your Minecraft server directory, and rename it to
`lazymc.exe`.

Open a terminal, go to the server directory, and make sure you can execute it:

```bash
.\lazymc --help
```

When `lazymc` is ready, set up the [configuration](./res/lazymc.toml) and start
it up:

```bash
# In your Minecraft server directory:

# Generate lazymc configuration
.\lazymc config generate

# Edit configuration
# Set the correct server address, directory and start command
notepad lazymc.toml

# Start lazymc
.\lazymc start
```

Before you use this in production, please ensure starting and stopping the
server works as expected by connecting to it once. Watch `lazymc`s output while
it starts and stops. If stopping results in errors, fix this first to prevent
corrupting world/user data.

Follow this repository with the _Watch_ button on the top right to be notified of new releases.

Everything should now be ready to go! Connect with your Minecraft client to wake
your server up!

_Note: if you put `lazymc` in `PATH`, or if you
[install](../README.md#compile-from-source) it through Cargo, you can invoke
`lazymc` everywhere directly without the `.\` prefix.

[latest-release]: https://github.com/timvisee/lazymc/releases/latest
