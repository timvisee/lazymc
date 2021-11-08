use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use minecraft_protocol::data::server_status::ServerStatus;
use tokio::process::Command;

use crate::config::SERVER_CMD;

/// Shared server state.
#[derive(Default, Debug)]
pub struct ServerState {
    /// Whether the server is online.
    online: AtomicBool,

    /// Whether the server is starting.
    starting: AtomicBool,

    /// Server PID.
    pid: Mutex<Option<u32>>,

    /// Last known server status.
    ///
    /// Once set, this will remain set, and isn't cleared when the server goes offline.
    status: Mutex<Option<ServerStatus>>,
}

impl ServerState {
    /// Whether the server is online.
    pub fn online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// Set whether the server is online.
    pub fn set_online(&self, online: bool) {
        self.online.store(online, Ordering::Relaxed)
    }

    /// Whether the server is starting.
    pub fn starting(&self) -> bool {
        self.starting.load(Ordering::Relaxed)
    }

    /// Set whether the server is starting.
    pub fn set_starting(&self, starting: bool) {
        self.starting.store(starting, Ordering::Relaxed)
    }

    /// Kill any running server.
    pub fn kill_server(&self) -> bool {
        if let Some(pid) = *self.pid.lock().unwrap() {
            warn!("Sending kill signal to server");
            kill_gracefully(pid);
            return true;
        }

        false
    }

    /// Set server PID.
    pub fn set_pid(&self, pid: Option<u32>) {
        *self.pid.lock().unwrap() = pid;
    }

    /// Update the server status.
    pub fn set_status(&self, status: ServerStatus) {
        self.status.lock().unwrap().replace(status);
    }
}

/// Start Minecraft server.
pub async fn start(state: Arc<ServerState>) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(SERVER_CMD);

    info!("Starting server...");
    let mut child = cmd.spawn()?;

    state.set_pid(Some(child.id().expect("unknown server PID")));

    let status = child.wait().await?;
    info!("Server stopped (status: {})\n", status);

    // Reset online and starting state
    // TODO: also set this when returning early due to error
    state.set_pid(None);
    state.set_online(false);
    state.set_starting(false);

    Ok(())
}

/// Gracefully kill process.
fn kill_gracefully(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGINT);
    }

    #[cfg(not(unix))]
    {
        // TODO: implement this for Windows
        unimplemented!();
    }
}
