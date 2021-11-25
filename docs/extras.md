# Extras

Some extra steps and recommendations when using lazymc:

Before you use this in production, always ensure starting and stopping the
server works as expected by connecting to it once. Watch lazymc's output while
it starts and stops. If stopping results in errors, fix this first to prevent
corrupting world/user data.

Follow this repository with the _Watch_ button on the top right to be notified
of new releases.

## Recommended

- [Protocol version](./protocol-version.md):
  _set correct Minecraft protocol version for the best client compatability_
- [Proxy IP](./proxy-ip.md):
  _fix incorrect client IPs on Minecraft server, notify server of correct IP with `PROXY` header_

## Tips

- [bash with start command](./command_bash.md):
  _how to properly use a bash script as server start command_

## Experimental features

- [Join method: lobby](./join-method-lobby.md):
  _keep clients in fake lobby world while server starts, teleport to real server when ready_
