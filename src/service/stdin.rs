use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use crate::config::Config;
use crate::server::Server;

#[cfg(feature = "rcon")]
use crate::mc::rcon::Rcon;

/// Service to read terminal input and send it to the server via RCON or signal handling.
pub async fn service(config: Arc<Config>, _server: Arc<Server>) {
    // Use `tokio::io::stdin` for asynchronous standard input handling
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        if line.trim().is_empty() {
            continue;
        }

        // Attempt to send via RCON if enabled
        #[cfg(feature = "rcon")]
        if config.rcon.enabled {
            if let Err(err) = send_rcon_command(&line, &config).await {
                eprintln!("Failed to send command via RCON: {}", err);
            }
        } else {
            // Handle other cases if RCON is not enabled
            eprintln!("RCON not enabled; alternative handling for command: {}", line);
        }
    }
}

#[cfg(feature = "rcon")]
async fn send_rcon_command(command: &str, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut rcon = Rcon::connect_config(config).await?;
    rcon.cmd(command).await?;
    rcon.close().await;
    Ok(())
}
