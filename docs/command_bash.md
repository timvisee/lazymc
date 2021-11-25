# Use bash script to start server

You may use a `bash` script to start your server rather than invoking `java`
directly. This requires some changes though to ensure your server properly shuts
down.

When lazymc stops your server it sends a [`SIGTERM`][sigterm] signal to the
invoked server process to gracefully shut it down. `bash` ignores this signal by
default and keeps the Minecraft server running.

You must configure `bash` to [forward][forward-signal] the signal to properly
shutdown the Minecraft server as well.

[sigterm]: https://en.wikipedia.org/wiki/Signal_(IPC)#SIGTERM
[forward-signal]: https://unix.stackexchange.com/a/434269/61092

## Example

Here's a minimal example, trapping the signal and forwarding it to the server.
Be sure to set the correct server JAR file and appropriate memory limits.

[`start-server`](../res/start-server):

```bash
#!/bin/bash

# Server JAR file, set this to your own
FILE=server.jar

# Trap SIGTERM, forward it to server process ID
trap 'kill -TERM $PID' TERM INT

# Start server
java -Xms1G -Xmx1G -jar $FILE --nogui &

# Remember server process ID, wait for it to quit, then reset the trap
PID=$!
wait $PID
trap - TERM INT
wait $PID
```
