use std::time::Duration;

use rust_rcon::{Connection, Error as RconError};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time;

use crate::config::Config;
use crate::proxy;

/// Minecraft RCON quirk.
///
/// Wait this time between RCON operations.
/// The Minecraft RCON implementation is very broken and brittle, this is used in the hopes to
/// improve reliability.
const QUIRK_RCON_GRACE_TIME: Duration = Duration::from_millis(200);

/// An RCON client.
pub struct Rcon {
    con: Connection<TcpStream>,
}

impl Rcon {
    /// Connect to a host.
    pub async fn connect(
        config: &Config,
        addr: &str,
        pass: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Connect to our TCP stream
        let mut stream = TcpStream::connect(addr).await?;

        // Add proxy header
        if config.rcon.send_proxy_v2 {
            trace!(target: "lazymc::rcon", "Sending local proxy header for RCON connection");
            stream.write_all(&proxy::local_proxy_header()?).await?;
        }

        // Start connection
        let con = Connection::builder()
            .enable_minecraft_quirks(true)
            .handshake(stream, pass)
            .await?;

        Ok(Self { con })
    }

    /// Connect to a host from the given configuration.
    pub async fn connect_config(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        // RCON address
        let mut addr = config.server.address;
        addr.set_port(config.rcon.port);
        let addr = addr.to_string();

        Self::connect(config, &addr, &config.rcon.password).await
    }

    /// Send command over RCON.
    pub async fn cmd(&mut self, cmd: &str) -> Result<String, RconError> {
        // Minecraft quirk
        time::sleep(QUIRK_RCON_GRACE_TIME).await;

        // Actually send RCON command
        debug!(target: "lazymc::rcon", "Sending RCON: {}", cmd);
        self.con.cmd(cmd).await
    }

    /// Close connection.
    pub async fn close(self) {
        // Minecraft quirk
        time::sleep(QUIRK_RCON_GRACE_TIME).await;
    }
}
