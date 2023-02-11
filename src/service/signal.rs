use std::sync::Arc;

use crate::server::{self, Server};
use crate::util::error;

/// Signal handler task.
pub async fn service(server: Arc<Server>) {
    loop {
        // Wait for SIGTERM/SIGINT signal
        tokio::signal::ctrl_c().await.unwrap();

        // Quit if stopped
        if server.state() == server::State::Stopped {
            quit();
        }

        // Try to stop server
        let stopping = server.stop().await;

        // If not stopping, maybe due to failure, just quit
        if !stopping {
            quit();
        }
    }
}

/// Gracefully quit.
fn quit() -> ! {
    // TODO: gracefully quit self
    error::quit();
}
