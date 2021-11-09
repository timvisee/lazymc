use rust_rcon::{Connection, Error as RconError};

/// An RCON client.
pub struct Rcon {
    con: Connection,
}

impl Rcon {
    /// Connect to a host.
    pub async fn connect(addr: &str, pass: &str) -> Result<Self, ()> {
        // Start connection
        let con = Connection::builder()
            .enable_minecraft_quirks(true)
            .connect(addr, pass)
            .await
            .map_err(|err| {
                dbg!(err);
                ()
            })?;

        Ok(Self { con })
    }

    /// Send command over RCON.
    pub async fn cmd(&mut self, cmd: &str) -> Result<String, RconError> {
        debug!(target: "lazymc::rcon", "Sending RCON: {}", cmd);
        self.con.cmd(cmd).await
    }
}
