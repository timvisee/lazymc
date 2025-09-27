use std::sync::Arc;
use tokio::signal;

use crate::config::Config;
use crate::server::{self, Server};
use crate::util::error;

/// Main signal handler task.
pub async fn service(config: Arc<Config>, server: Arc<Server>) {
    loop {
        // Wait for SIGTERM/SIGINT signal
        signal::ctrl_c().await.unwrap();
        
        // Call the shutdown function
        shutdown(&config, &server).await;
    }
}

/// Shutdown the server gracefully, can be called from other modules.
pub async fn shutdown(config: &Arc<Config>, server: &Arc<Server>) {
    // Quit immediately if the server is already stopped
    if server.state() == server::State::Stopped {
        quit();
    }

    // Try to stop the server gracefully
    let stopping = server.stop(config).await;

    // If stopping fails, quit immediately
    if !stopping {
        quit();
    }
}

/// Gracefully quit the application.
fn quit() -> ! {
    // TODO: gracefully quit self
    error::quit();
}
