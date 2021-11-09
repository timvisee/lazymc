use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::FutureExt;
use minecraft_protocol::data::server_status::ServerStatus;
use tokio::process::Command;

use crate::config::Config;

/// Server state.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum State {
    /// Server is starting.
    Starting,

    /// Server is online and responding.
    Started,

    /// Server is stopping.
    Stopping,

    /// Server is stopped.
    Stopped,
}

/// Shared server state.
#[derive(Debug)]
pub struct Server {
    /// Server state.
    state: Mutex<State>,

    /// Server process PID.
    ///
    /// Set if a server process is running.
    pid: Mutex<Option<u32>>,

    /// Last known server status.
    ///
    /// Will remain set once known, not cleared if server goes offline.
    status: Mutex<Option<ServerStatus>>,

    /// Last active time.
    ///
    /// The last time there was activity on the server. Also set at the moment the server comes
    /// online.
    last_active: Mutex<Option<Instant>>,

    /// Force server to stay online until.
    keep_online_until: Mutex<Option<Instant>>,
}

impl Server {
    /// Get current state.
    pub fn state(&self) -> State {
        *self.state.lock().unwrap()
    }

    /// Set a new state.
    ///
    /// This updates various other internal things depending on how the state changes.
    ///
    /// Returns false if the state didn't change, in which case nothing happens.
    fn update_state(&self, state: State, config: &Config) -> bool {
        self.update_state_from(None, state, config)
    }

    /// Set new state, from a current state.
    ///
    /// This updates various other internal things depending on how the state changes.
    ///
    /// Returns false if current state didn't match `from` or if nothing changed.
    fn update_state_from(&self, from: Option<State>, state: State, config: &Config) -> bool {
        // Get current state, must differ from current, must match from
        let mut cur = self.state.lock().unwrap();
        if *cur == state || (from.is_some() && from != Some(*cur)) {
            return false;
        }

        trace!("Change server state from {:?} to {:?}", *cur, state);

        // Online/offline messages
        match state {
            State::Started => info!(target: "lazymc::monitor", "Server is now online"),
            State::Stopped => info!(target: "lazymc::monitor", "Server is now sleeping"),
            _ => {}
        }

        // If Starting -> Started, update active time and keep it online for configured time
        if *cur == State::Starting && state == State::Started {
            self.update_last_active();
            self.keep_online_for(Some(config.time.min_online_time));
        }

        *cur = state;
        true
    }

    /// Update status as polled from the server.
    ///
    /// This updates various other internal things depending on the current state and the given
    /// status.
    pub fn update_status(&self, config: &Config, status: Option<ServerStatus>) {
        let state = *self.state.lock().unwrap();

        // Update state based on curren
        match (state, &status) {
            (State::Stopped | State::Starting, Some(_)) => {
                self.update_state(State::Started, config);
            }
            (State::Started, None) => {
                self.update_state(State::Stopped, config);
            }
            _ => {}
        }

        // Update last status if known
        if let Some(status) = status {
            // Update last active time if there are online players
            if status.players.online > 0 {
                self.update_last_active();
            }

            self.status.lock().unwrap().replace(status);
        }
    }

    /// Try to start the server.
    ///
    /// Does nothing if currently not in stopped state.
    pub fn start(config: Arc<Config>, server: Arc<Server>, username: Option<String>) -> bool {
        // Must set state from stopped to starting
        if !server.update_state_from(Some(State::Stopped), State::Starting, &config) {
            return false;
        }

        // Log starting message
        match username {
            Some(username) => info!(target: "lazymc", "Starting server for '{}'...", username),
            None => info!(target: "lazymc", "Starting server..."),
        }

        // Invoke server command in separate task
        tokio::spawn(invoke_server_cmd(config, server).map(|_| ()));
        true
    }

    /// Stop running server.
    ///
    /// This requires the server PID to be known.
    #[allow(unused_variables)]
    pub async fn stop(&self, config: &Config) -> bool {
        // We must have a running process
        let has_process = self.pid.lock().unwrap().is_some();
        if !has_process {
            return false;
        }

        // Try to stop through RCON if started
        #[cfg(feature = "rcon")]
        if self.state() == State::Started && stop_server_rcon(config, &self).await {
            return true;
        }

        // Try to stop through signal
        #[cfg(unix)]
        if stop_server_signal(config, &self) {
            return true;
        }

        false
    }

    /// Decide whether the server should sleep.
    ///
    /// Always returns false if it is currently not online.
    pub fn should_sleep(&self, config: &Config) -> bool {
        // Server must be online
        if *self.state.lock().unwrap() != State::Started {
            return false;
        }

        // Never sleep if players are online
        let players_online = self
            .status
            .lock()
            .unwrap()
            .as_ref()
            .map(|status| status.players.online > 0)
            .unwrap_or(false);
        if players_online {
            trace!(target: "lazymc", "Not sleeping because players are online");
            return false;
        }

        // Don't sleep when keep online until isn't expired
        let keep_online = self
            .keep_online_until
            .lock()
            .unwrap()
            .map(|i| i >= Instant::now())
            .unwrap_or(false);
        if keep_online {
            trace!(target: "lazymc", "Not sleeping because of keep online");
            return false;
        }

        // Last active time must have passed sleep threshold
        if let Some(last_idle) = self.last_active.lock().unwrap().as_ref() {
            return last_idle.elapsed() >= Duration::from_secs(config.time.sleep_after as u64);
        }

        false
    }

    /// Clone last known server status.
    // TODO: return mutex guard here
    pub fn clone_status(&self) -> Option<ServerStatus> {
        self.status.lock().unwrap().clone()
    }

    /// Update the last active time.
    fn update_last_active(&self) {
        self.last_active.lock().unwrap().replace(Instant::now());
    }

    /// Force the server to be online for the given number of seconds.
    fn keep_online_for(&self, duration: Option<u32>) {
        *self.keep_online_until.lock().unwrap() = duration
            .filter(|d| *d > 0)
            .map(|d| Instant::now() + Duration::from_secs(d as u64));
    }
}

impl Default for Server {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::Stopped),
            pid: Default::default(),
            status: Default::default(),
            last_active: Default::default(),
            keep_online_until: Default::default(),
        }
    }
}

/// Invoke server command, store PID and wait for it to quit.
pub async fn invoke_server_cmd(
    config: Arc<Config>,
    state: Arc<Server>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Build command
    let args = shlex::split(&config.server.command).expect("invalid server command");
    let mut cmd = Command::new(&args[0]);
    cmd.args(args.iter().skip(1));
    cmd.kill_on_drop(true);

    // Set working directory
    if let Some(ref dir) = config.server.directory {
        cmd.current_dir(dir);
    }

    // Spawn process
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            error!(target: "lazymc", "Failed to start server process through command");
            return Err(err.into());
        }
    };

    // Remember PID
    state
        .pid
        .lock()
        .unwrap()
        .replace(child.id().expect("unknown server PID"));

    // Wait for process to exit, handle status
    match child.wait().await {
        Ok(status) if status.success() => {
            debug!(target: "lazymc", "Server process stopped successfully ({})", status);
        }
        Ok(status) => {
            warn!(target: "lazymc", "Server process stopped with error code ({})", status);
        }
        Err(err) => {
            error!(target: "lazymc", "Failed to wait for server process to quit: {}", err);
            error!(target: "lazymc", "Assuming server quit, cleaning up...");
        }
    }

    // Set state to stopped, update server PID
    state.pid.lock().unwrap().take();
    state.update_state(State::Stopped, &config);

    Ok(())
}

/// Stop server through RCON.
#[cfg(feature = "rcon")]
async fn stop_server_rcon(config: &Config, server: &Server) -> bool {
    use crate::mc::rcon::Rcon;

    // RCON must be enabled
    if !config.rcon.enabled {
        return false;
    }

    // RCON address
    let mut addr = config.server.address.clone();
    addr.set_port(config.rcon.port);
    let addr = addr.to_string();

    // Create RCON client
    let mut rcon = match Rcon::connect(&addr, &config.rcon.password).await {
        Ok(rcon) => rcon,
        Err(err) => {
            error!(target: "lazymc", "Failed to RCON server to sleep: {}", err);
            return false;
        }
    };

    // Invoke stop
    if let Err(err) = rcon.cmd("stop").await {
        error!(target: "lazymc", "Failed to invoke stop through RCON: {}", err);
    }

    // Set server to stopping state
    // TODO: set before stop command, revert state on failure
    server.update_state(State::Stopping, config);

    true
}

/// Stop server by sending SIGTERM signal.
///
/// Only available on Unix.
#[cfg(unix)]
fn stop_server_signal(config: &Config, server: &Server) -> bool {
    // Grab PID
    let pid = match *server.pid.lock().unwrap() {
        Some(pid) => pid,
        None => return false,
    };

    // Set stopping state, send kill signal
    // TODO: revert state on failure
    server.update_state(State::Stopping, config);
    crate::os::kill_gracefully(pid);

    true
}
