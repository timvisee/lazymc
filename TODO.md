# TODO

- Better organize code
- Resolve TODOs in code
- Don't drop errors, handle everywhere where needed (some were dropped while
  prototyping to speed up development)

## Nice to have

- Use server whitelist/blacklist
- Console error if server already started on port, not through `lazymc`
- Kick with message if proxy-to-server connection fails for new client.
- Test configuration on start (server dir exists, command not empty)
- Dynamically increase/decrease server polling interval based on server state
- Server polling through query (`enable-query` in `server.properties`, uses GameSpy4 protocol)

## Experiment

- `io_uring` on Linux for efficient proxying (see `tokio-uring`)

## Lobby join method

- add support for more Minecraft versions (with changed protocols)
- support online mode (encryption)
- hold back packets (whitelist), forward to server at connect before joining
- add support for forge (emulate mod list communication)
- on login plugin request during login state, respond with empty payload, not supported
