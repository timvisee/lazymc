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

When lazymc is ready, set up the [configuration](../res/lazymc.toml) and start it
up:

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

Please see [extras](./extras.md) for recommendations and additional things
to set up (e.g. how to fix incorrect client IPs and IP banning on your server).

After you've read through the [extras](./extras.md), everything should now
be ready to go! Connect with your Minecraft client to wake your server up!

_Note: if you put `lazymc` in `PATH`, or if you
[install](../README.md#compile-from-source) it through Cargo, you can invoke
`lazymc` everywhere directly without the `.\` prefix._

[latest-release]: https://github.com/timvisee/lazymc/releases/latest
