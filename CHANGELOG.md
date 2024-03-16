# Changelog

## 0.2.11 (2024-03-16)

- Add support for Minecraft 1.20.3 and 1.20.4
- Improve error handling of parsing server favicon
- Fix typo in log message
- Update dependencies

## 0.2.10 (2023-02-20)

- Do not report an error when server exits with status code 143

## 0.2.9 (2023-02-14)

- Fix dropping all connections when `server.drop_banned_ips` was enabled
- Update dependencies

## 0.2.8 (2023-01-30)

- Add `freeze_process` feature on Unix platforms to freeze a sleeping server
  rather than shutting it down.
- Update default Minecraft version to 1.19.3
- Remove macOS builds from releases, users can compile from source
- Update dependencies

## 0.2.7 (2021-12-13)

- Update default Minecraft version to 1.18.1
- Update dependencies

## 0.2.6 (2021-11-28)

- Add whitelist support, use server whitelist to prevent unknown users from waking server
- Update dependencies

## 0.2.5 (2021-11-25)

- Add support Minecraft 1.16.3 to 1.17.1 with lobby join method
- Add support for Forge client/server to lobby join method (partial)
- Probe server on start with fake user to fetch server settings improving compatibility
- Improve lobby compatibility, send probed server data to client when possible
- Skip lobby join method if server probe is not yet finished
- Generate lobby dimension configuration on the fly based on server dimensions
- Fix unsupported lobby dimension configuration values for some Minecraft versions
- Demote IP ban list reload message from info to debug
- Update dependencies

## 0.2.4 (2021-11-24)

- Fix status response issues with missing server icon, fall back to default icon
- Fix incorrect UUID for players in lobby logic
- Make server directory relative to configuration file path
- Assume SIGTERM exit code for server process to be successful on Unix
- Update features in README
- Update dependencies

## 0.2.3 (2021-11-22)

- Add support for `PROXY` header to notify Minecraft server of real client IP
- Only enable RCON by default on Windows
- Update dependencies

## 0.2.2 (2021-11-18)

- Add server favicon to status response

## 0.2.1 (2021-11-17)

- Add support for using host names in config address fields
- Handle banned players within `lazymc` based on server `banned-ips.json`
- Update dependencies

## 0.2.0 (2021-11-15)

- Add lockout feature, enable to kick all connecting clients with a message
- Add option to configure list of join methods to occupy client with while server is starting (kick, hold, forward, lobby)
- Add lobby join method, keeps client in lobby world on emulated server, teleports to real server when it is ready (highly experimental)
- Add forward join method to forward (proxy) client to other host while server is starting
- Restructure `lazymc.toml` configuration
- Increase packet reading buffer size to speed things up
- Add support for Minecraft packet compression
- Show warning if config version is outdated or invalid
- Various fixes and improvements

## 0.1.3 (2021-11-15)

- Fix binary release

## 0.1.2 (2021-11-15)

- Add Linux ARMv7 and aarch64 releases
- RCON now works if server is running while server command already quit
- Various RCON tweaks in an attempt to make it more robust and reliable (cooldown, exclusive lock, invocation spacing)
- Increase server monitoring timeout to 20 seconds
- Improve waiting for server logic when holding client
- Various fixes and improvements

## 0.1.1 (2021-11-14)

- Make server sleeping errors more descriptive
- Add server quit cooldown period, intended to prevent RCON errors due to RCON
  server thread something quitting after main server
- Rewrite `enable-status = true` in `server.properties`
- Rewrite `prevent-proxy-connections = false` in `server.properties` if
  Minecraft server has non-loopback address (other public IP)
- Add compile from source instructions to README
- Add Windows instructions to README
- Update dependencies
- Various fixes and improvements

## 0.1.0 (2021-11-11)

- Initial release
