use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use crate::config::Config;
use crate::server::Server;
use crate::service::signal::shutdown;

/// Service to read terminal input and send it to the server via piped stdin or RCON handling.
pub async fn service(config: Arc<Config>, server: Arc<Server>) {
    // Use `tokio::io::stdin` for asynchronous standard input handling
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        match trimmed_line.to_ascii_lowercase().as_str() {
            // Quit command
            "!quit" | "!exit" => {
                info!("Received quit command");
                shutdown(&config, &server).await;
                break;
            }

            // Start the server
            "!start" => {
                info!("Received start command");
                Server::start(config.clone(), server.clone(), None).await;
            }

            // Stop the server
            "!stop" => {
                info!("Received stop command");
                server.stop(&config).await;
            }

            // Any other command is sent to the Minecraft server's stdin
            command => {
                info!("Sending command to Minecraft server: {}", command);
                if let Err(e) = server.send_command(command).await {
                    eprintln!("Failed to send command to server: {}", e);
                }
            }
        }
    }
}
