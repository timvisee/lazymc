# Changelog

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
