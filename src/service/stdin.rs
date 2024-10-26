use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use crate::config::Config;
use crate::server::Server;
use crate::service::signal::shutdown;

/// Service to read terminal input and send it to the server via RCON or signal handling.
pub async fn service(config: Arc<Config>, server: Arc<Server>) {
    // Use `tokio::io::stdin` for asynchronous standard input handling
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        // Check for quit command
        if trimmed_line.eq_ignore_ascii_case("!quit") || trimmed_line.eq_ignore_ascii_case("!exit") {
            info!("Received quit command");
            // Gracefully shutdown
            shutdown(&config, &server).await;
            break;
        }

        // If !start command, start the server
        if trimmed_line.eq_ignore_ascii_case("!start") {
            info!("Received start command");
            Server::start(config.clone(), server.clone(), None).await;
            continue;
        }

        // If !stop command, stop the server
        if trimmed_line.eq_ignore_ascii_case("!stop") {
            info!("Received stop command");
            server.stop(&config).await;
            continue;
        }
        
    }
}

