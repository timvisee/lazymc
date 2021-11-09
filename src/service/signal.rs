use std::sync::Arc;

use crate::config::Config;
use crate::server::ServerState;

/// Signal handler task.
pub async fn service(config: Arc<Config>, server_state: Arc<ServerState>) {
    loop {
        // Wait for SIGTERM/SIGINT signal
        tokio::signal::ctrl_c().await.unwrap();

        // Attemp to kill server
        let killed = !server_state.kill_server(&config).await;

        // If we don't kill the server, quit this process
        if !killed {
            // TODO: gracefully kill itself instead
            std::process::exit(1)
        }
    }
}
