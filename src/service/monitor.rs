use std::sync::Arc;

use crate::monitor;
use crate::server::Server;

/// Server monitor task.
pub async fn service(server: Arc<Server>) {
    monitor::monitor_server(server).await
}
