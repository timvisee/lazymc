use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use minecraft_protocol::data::server_status::ServerStatus;
use tokio::process::Command;

use crate::config::{SERVER_CMD, SLEEP_DELAY};

/// Shared server state.
#[derive(Default, Debug)]
pub struct ServerState {
    /// Whether the server is online.
    online: AtomicBool,

    /// Whether the server is starting.
    // TODO: use enum for starting/started/stopping states
    starting: AtomicBool,

    /// Whether the server is stopping.
    stopping: AtomicBool,

    /// Server PID.
    pid: Mutex<Option<u32>>,

    /// Last known server status.
    ///
    /// Once set, this will remain set, and isn't cleared when the server goes offline.
    // TODO: make this private?
    pub status: Mutex<Option<ServerStatus>>,

    /// Last active time.
    ///
    /// The last known time when the server was active with online players.
    last_active: Mutex<Option<Instant>>,
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
            debug!("Sending kill signal to server");
            kill_gracefully(pid);

            // TODO: should we set this?
            self.set_online(false);

            return true;
        }

        // TODO: set stopping state elsewhere
        self.stopping.store(true, Ordering::Relaxed);

        false
    }

    /// Set server PID.
    pub fn set_pid(&self, pid: Option<u32>) {
        *self.pid.lock().unwrap() = pid;
    }

    /// Clone the last known server status.
    pub fn clone_status(&self) -> Option<ServerStatus> {
        self.status.lock().unwrap().clone()
    }

    /// Update the server status.
    pub fn set_status(&self, status: ServerStatus) {
        self.status.lock().unwrap().replace(status);
    }

    /// Update the server status, online state and last active time.
    // TODO: clean this up
    pub fn update_status(&self, status: Option<ServerStatus>) {
        let stopping = self.stopping.load(Ordering::Relaxed);
        let was_online = self.online();
        let online = status.is_some() && !stopping;
        self.set_online(online);

        // If server just came online, update last active time
        if !was_online && online {
            // TODO: move this somewhere else
            info!("Server is now online");
            self.update_last_active_time();
        }

        // // If server just went offline, reset stopping state
        // // TODO: do this elsewhere
        // if stopping && was_online && !online {
        //     self.stopping.store(false, Ordering::Relaxed);
        // }

        if let Some(status) = status {
            // Update last active time if there are online players
            if status.players.online > 0 {
                self.update_last_active_time();
            }

            // Update last known players
            self.set_status(status);
        }
    }

    /// Update the last active time.
    pub fn update_last_active_time(&self) {
        self.last_active.lock().unwrap().replace(Instant::now());
    }

    /// Check whether the server should now sleep.
    pub fn should_sleep(&self) -> bool {
        // TODO: when initating server start, set last active time!
        // TODO: do not initiate sleep when starting?
        // TODO: do not initiate sleep when already initiated (with timeout)

        // Server must be online, and must not be starting
        if !self.online() || !self.starting() {
            return false;
        }

        // Last active time must have passed sleep threshold
        if let Some(last_idle) = self.last_active.lock().unwrap().as_ref() {
            return last_idle.elapsed() >= Duration::from_secs(SLEEP_DELAY);
        }

        false
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
    state.stopping.store(false, Ordering::Relaxed);

    Ok(())
}

/// Gracefully kill process.
fn kill_gracefully(pid: u32) {
    #[cfg(unix)]
    unsafe {
        debug!("Sending SIGTERM signal to {} to kill server", pid);
        let result = libc::kill(pid as i32, libc::SIGTERM);
        trace!("SIGTERM result: {}", result);

        // TODO: send sigterm to childs as well?
        // TODO: handle error if != 0
    }

    // TODO: implement for Windows
    #[cfg(not(unix))]
    {
        // TODO: implement this for Windows
        unimplemented!();
    }
}
