use std::time::Duration;

use rust_rcon::{Connection, Error as RconError};
use tokio::time;

/// Minecraft RCON quirk.
///
/// Wait this time between RCON operations.
/// The Minecraft RCON implementation is very broken and brittle, this is used in the hopes to
/// improve reliability.
const QUIRK_RCON_GRACE_TIME: Duration = Duration::from_millis(200);

/// An RCON client.
pub struct Rcon {
    con: Connection,
}

impl Rcon {
    /// Connect to a host.
    pub async fn connect(addr: &str, pass: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Start connection
        let con = Connection::builder()
            .enable_minecraft_quirks(true)
            .connect(addr, pass)
            .await?;

        Ok(Self { con })
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
