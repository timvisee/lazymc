# Changelog

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
