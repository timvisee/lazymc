use std::sync::Arc;

use crate::server::ServerState;

/// Signal handler task.
pub async fn service(server_state: Arc<ServerState>) {
    loop {
        tokio::signal::ctrl_c().await.unwrap();
        if !server_state.kill_server() {
            // TODO: gracefully kill itself instead
            std::process::exit(1)
        }
    }
}
