use std::sync::Arc;

use crate::config::Config;
use crate::monitor;
use crate::server::ServerState;

/// Server monitor task.
pub async fn service(config: Arc<Config>, state: Arc<ServerState>) {
    monitor::monitor_server(config, state).await
}
