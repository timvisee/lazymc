# TODO

- Better organize code
- Resolve TODOs in code
- Don't drop errors, handle everywhere where needed (some were dropped while
  prototyping to speed up development)

## Nice to have

- Console error if server already started on port, not through `lazymc`
- Kick with message if proxy-to-server connection fails for new client. 
- Test configuration on start (server dir exists, command not empty)
- Also quit `lazymc` after CTRL+C signal, after server has stopped
- Dynamically increase/decrease server polling interval based on server state
- Server polling through query (`enable-query` in `server.properties`, uses GameSpy4 protocol)

## Experiment

- Lobby method: let players connect with an emulated empty server (like 2b2t's
  queue), redirect them when the server started.
- `io_uring` on Linux for efficient proxying (see `tokio-uring`)
