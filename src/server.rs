use std::net::IpAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::FutureExt;
use minecraft_protocol::version::v1_20_3::status::ServerStatus;
use tokio::process::Command;
use tokio::sync::watch;
#[cfg(feature = "rcon")]
use tokio::sync::Semaphore;
use tokio::sync::{Mutex, RwLock, RwLockReadGuard};
use tokio::time;

use crate::config::{Config, Server as ConfigServer};
use crate::mc::ban::{BannedIp, BannedIps};
use crate::mc::whitelist::Whitelist;
use crate::os;
use crate::proto::packets::play::join_game::JoinGameData;

/// Server cooldown after the process quit.
/// Used to give it some more time to quit forgotten threads, such as for RCON.
const SERVER_QUIT_COOLDOWN: Duration = Duration::from_millis(2500);

/// RCON cooldown. Required period between RCON invocations.
///
/// The Minecraft RCON implementation is very broken and brittle, this is used in the hopes to
/// improve reliability.
#[cfg(feature = "rcon")]
const RCON_COOLDOWN: Duration = Duration::from_secs(15);

/// Exit codes that are allowed.
///
/// - 143: https://github.com/timvisee/lazymc/issues/26#issuecomment-1435670029
/// - 130: https://unix.stackexchange.com/q/386836/61092
const ALLOWED_EXIT_CODES: [i32; 2] = [130, 143];

/// Shared server state.
#[derive(Debug)]
pub struct Server {
    /// Server state.
    ///
    /// Matches `State`, utilzes AtomicU8 for better performance.
    state: AtomicU8,

    /// State watch sender, broadcast state changes.
    state_watch_sender: watch::Sender<State>,

    /// State watch receiver, subscribe to state changes.
    state_watch_receiver: watch::Receiver<State>,

    /// Server process PID.
    ///
    /// Set if a server process is running.
    pid: Mutex<Option<u32>>,

    /// Last known server status.
    ///
    /// Will remain set once known, not cleared if server goes offline.
    status: RwLock<Option<ServerStatus>>,

    /// Last active time.
    ///
    /// The last time there was activity on the server. Also set at the moment the server comes
    /// online.
    last_active: RwLock<Option<Instant>>,

    /// Force server to stay online until.
    keep_online_until: RwLock<Option<Instant>>,

    /// Time to force kill the server process at.
    ///
    /// Used as starting/stopping timeout.
    kill_at: RwLock<Option<Instant>>,

    /// List of banned IPs.
    banned_ips: RwLock<BannedIps>,

    /// Whitelist if enabled.
    whitelist: RwLock<Option<Whitelist>>,

    /// Lock for exclusive RCON operations.
    #[cfg(feature = "rcon")]
    rcon_lock: Semaphore,

    /// Last time server was stopped over RCON.
    #[cfg(feature = "rcon")]
    rcon_last_stop: Mutex<Option<Instant>>,

    /// Probed join game data.
    pub probed_join_game: RwLock<Option<JoinGameData>>,

    /// Forge payload.
    ///
    /// Sent to clients when they connect to lobby. Recorded from server by probe.
    pub forge_payload: RwLock<Vec<Vec<u8>>>,
}

impl Server {
    /// Get current state.
    pub fn state(&self) -> State {
        State::from_u8(self.state.load(Ordering::Relaxed))
    }

    /// Get state receiver to subscribe on server state changes.
    pub fn state_receiver(&self) -> watch::Receiver<State> {
        self.state_watch_receiver.clone()
    }

    /// Set a new state.
    ///
    /// This updates various other internal things depending on how the state changes.
    ///
    /// Returns false if the state didn't change, in which case nothing happens.
    async fn update_state(&self, state: State, config: &Config) -> bool {
        self.update_state_from(None, state, config).await
    }

    /// Set new state, from a current state.
    ///
    /// This updates various other internal things depending on how the state changes.
    ///
    /// Returns false if current state didn't match `from` or if nothing changed.
    async fn update_state_from(&self, from: Option<State>, new: State, config: &Config) -> bool {
        // Atomically swap state to new, return if from doesn't match
        let old = State::from_u8(match from {
            Some(from) => match self.state.compare_exchange(
                from.to_u8(),
                new.to_u8(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(old) => old,
                Err(_) => return false,
            },
            None => self.state.swap(new.to_u8(), Ordering::Relaxed),
        });

        // State must be changed
        if old == new {
            return false;
        }

        trace!("Change server state from {:?} to {:?}", old, new);

        // Broadcast change
        let _ = self.state_watch_sender.send(new);

        // Update kill at time for starting/stopping state
        *self.kill_at.write().await = match new {
            State::Starting if config.server.start_timeout > 0 => {
                Some(Instant::now() + Duration::from_secs(config.server.start_timeout as u64))
            }
            State::Stopping if config.server.stop_timeout > 0 => {
                Some(Instant::now() + Duration::from_secs(config.server.stop_timeout as u64))
            }
            _ => None,
        };

        // Online/offline messages
        match new {
            State::Started => info!(target: "lazymc::monitor", "Server is now online"),
            State::Stopped => info!(target: "lazymc::monitor", "Server is now sleeping"),
            _ => {}
        }

        // If Starting -> Started, update active time and keep it online for configured time
        if old == State::Starting && new == State::Started {
            self.update_last_active().await;
            self.keep_online_for(Some(config.time.min_online_time))
                .await;
        }

        true
    }

    /// Update status as obtained from the server.
    ///
    /// This updates various other internal things depending on the current state and the given
    /// status.
    pub async fn update_status(&self, config: &Config, status: Option<ServerStatus>) {
        // Update state based on curren
        match (self.state(), &status) {
            (State::Stopped | State::Starting, Some(_)) => {
                self.update_state(State::Started, config).await;
            }
            (State::Started, None) => {
                self.update_state(State::Stopped, config).await;
            }
            _ => {}
        }

        // Update last status if known
        if let Some(status) = status {
            // Update last active time if there are online players
            if status.players.online > 0 {
                self.update_last_active().await;
            }

            self.status.write().await.replace(status);
        }
    }

    /// Try to start the server.
    ///
    /// Does nothing if currently not in stopped state.
    pub async fn start(config: Arc<Config>, server: Arc<Server>, username: Option<String>) -> bool {
        // Must set state from stopped to starting
        if !server
            .update_state_from(Some(State::Stopped), State::Starting, &config)
            .await
        {
            return false;
        }

        // Log starting message
        match username {
            Some(username) => info!(target: "lazymc", "Starting server for '{}'...", username),
            None => info!(target: "lazymc", "Starting server..."),
        }

        // Unfreeze server if it is frozen
        #[cfg(unix)]
        if config.server.freeze_process && unfreeze_server_signal(&config, &server).await {
            return true;
        }

        // Spawn server in new task
        Self::spawn_server_task(config, server);
        true
    }

    /// Spawn the server task.
    ///
    /// This should not be called directly.
    fn spawn_server_task(config: Arc<Config>, server: Arc<Server>) {
        tokio::spawn(invoke_server_cmd(config, server).map(|_| ()));
    }

    /// Stop running server.
    ///
    /// This will attempt to stop the server with all available methods.
    #[allow(unused_variables)]
    pub async fn stop(&self, config: &Config) -> bool {
        // Try to freeze through signal
        #[cfg(unix)]
        if config.server.freeze_process && freeze_server_signal(config, self).await {
            return true;
        }

        // Try to stop through RCON if started
        #[cfg(feature = "rcon")]
        if self.state() == State::Started && stop_server_rcon(config, self).await {
            return true;
        }

        // Try to stop through signal
        #[cfg(unix)]
        if stop_server_signal(config, self).await {
            return true;
        }

        warn!(target: "lazymc", "Failed to stop server, no more suitable stopping method to use");
        false
    }

    /// Force kill running server.
    ///
    /// This requires the server PID to be known.
    pub async fn force_kill(&self) -> bool {
        if let Some(pid) = *self.pid.lock().await {
            return os::force_kill(pid);
        }
        false
    }

    /// Decide whether the server should sleep.
    ///
    /// Always returns false if it is currently not online.
    pub async fn should_sleep(&self, config: &Config) -> bool {
        // Server must be online
        if self.state() != State::Started {
            return false;
        }

        // Never sleep if players are online
        let players_online = self
            .status
            .read()
            .await
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
            .read()
            .await
            .map(|i| i >= Instant::now())
            .unwrap_or(false);
        if keep_online {
            trace!(target: "lazymc", "Not sleeping because of keep online");
            return false;
        }

        // Last active time must have passed sleep threshold
        if let Some(last_idle) = self.last_active.read().await.as_ref() {
            return last_idle.elapsed() >= Duration::from_secs(config.time.sleep_after as u64);
        }

        false
    }

    /// Decide whether to force kill the server process.
    pub async fn should_kill(&self) -> bool {
        self.kill_at
            .read()
            .await
            .map(|t| t <= Instant::now())
            .unwrap_or(false)
    }

    /// Read last known server status.
    pub async fn status(&self) -> RwLockReadGuard<'_, Option<ServerStatus>> {
        self.status.read().await
    }

    /// Update the last active time.
    async fn update_last_active(&self) {
        self.last_active.write().await.replace(Instant::now());
    }

    /// Force the server to be online for the given number of seconds.
    async fn keep_online_for(&self, duration: Option<u32>) {
        *self.keep_online_until.write().await = duration
            .filter(|d| *d > 0)
            .map(|d| Instant::now() + Duration::from_secs(d as u64));
    }

    /// Check whether the given IP is banned.
    ///
    /// This uses the latest known `banned-ips.json` contents if known.
    /// If this feature is disabled, this will always return false.
    pub async fn is_banned_ip(&self, ip: &IpAddr) -> bool {
        self.banned_ips.read().await.is_banned(ip)
    }

    /// Get user ban entry.
    pub async fn ban_entry(&self, ip: &IpAddr) -> Option<BannedIp> {
        self.banned_ips.read().await.get(ip)
    }

    /// Check whether the given IP is banned.
    ///
    /// This uses the latest known `banned-ips.json` contents if known.
    /// If this feature is disabled, this will always return false.
    pub fn is_banned_ip_blocking(&self, ip: &IpAddr) -> bool {
        futures::executor::block_on(async { self.is_banned_ip(ip).await })
    }

    /// Check whether the given username is whitelisted.
    ///
    /// Returns `true` if no whitelist is currently used.
    pub async fn is_whitelisted(&self, username: &str) -> bool {
        self.whitelist
            .read()
            .await
            .as_ref()
            .map(|w| w.is_whitelisted(username))
            .unwrap_or(true)
    }

    /// Update the list of banned IPs.
    pub async fn set_banned_ips(&self, ips: BannedIps) {
        *self.banned_ips.write().await = ips;
    }

    /// Update the list of banned IPs.
    pub fn set_banned_ips_blocking(&self, ips: BannedIps) {
        futures::executor::block_on(async { self.set_banned_ips(ips).await })
    }

    /// Update the whitelist.
    pub async fn set_whitelist(&self, whitelist: Option<Whitelist>) {
        *self.whitelist.write().await = whitelist;
    }

    /// Update the whitelist.
    pub fn set_whitelist_blocking(&self, whitelist: Option<Whitelist>) {
        futures::executor::block_on(async { self.set_whitelist(whitelist).await })
    }
}

impl Default for Server {
    fn default() -> Self {
        let (state_watch_sender, state_watch_receiver) = watch::channel(State::Stopped);

        Self {
            state: AtomicU8::new(State::Stopped.to_u8()),
            state_watch_sender,
            state_watch_receiver,
            pid: Default::default(),
            status: Default::default(),
            last_active: Default::default(),
            keep_online_until: Default::default(),
            kill_at: Default::default(),
            banned_ips: Default::default(),
            whitelist: Default::default(),
            #[cfg(feature = "rcon")]
            rcon_lock: Semaphore::new(1),
            #[cfg(feature = "rcon")]
            rcon_last_stop: Default::default(),
            probed_join_game: Default::default(),
            forge_payload: Default::default(),
        }
    }
}

/// Server state.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum State {
    /// Server is stopped.
    Stopped,

    /// Server is starting.
    Starting,

    /// Server is online and responding.
    Started,

    /// Server is stopping.
    Stopping,
}

impl State {
    /// From u8, panics if invalid.
    pub fn from_u8(state: u8) -> Self {
        match state {
            0 => Self::Stopped,
            1 => Self::Starting,
            2 => Self::Started,
            3 => Self::Stopping,
            _ => panic!("invalid State u8"),
        }
    }

    /// To u8.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Stopped => 0,
            Self::Starting => 1,
            Self::Started => 2,
            Self::Stopping => 3,
        }
    }
}

/// Invoke server command, store PID and wait for it to quit.
pub async fn invoke_server_cmd(
    config: Arc<Config>,
    state: Arc<Server>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Configure command
    let args = shlex::split(&config.server.command).expect("invalid server command");
    let mut cmd = Command::new(&args[0]);
    cmd.args(args.iter().skip(1));
    cmd.kill_on_drop(true);

    // Set working directory
    if let Some(ref dir) = ConfigServer::server_directory(&config) {
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
        .await
        .replace(child.id().expect("unknown server PID"));

    // Wait for process to exit, handle status
    let crashed = match child.wait().await {
        Ok(status) if status.success() => {
            debug!(target: "lazymc", "Server process stopped successfully ({})", status);
            false
        }
        Ok(status)
            if status
                .code()
                .map(|ref code| ALLOWED_EXIT_CODES.contains(code))
                .unwrap_or(false) =>
        {
            debug!(target: "lazymc", "Server process stopped successfully by SIGTERM ({})", status);
            false
        }
        Ok(status) => {
            warn!(target: "lazymc", "Server process stopped with error code ({})", status);
            state.state() == State::Started
        }
        Err(err) => {
            error!(target: "lazymc", "Failed to wait for server process to quit: {}", err);
            error!(target: "lazymc", "Assuming server quit, cleaning up...");
            false
        }
    };

    // Forget server PID
    state.pid.lock().await.take();

    // Give server a little more time to quit forgotten threads
    time::sleep(SERVER_QUIT_COOLDOWN).await;

    // Set server state to stopped
    state.update_state(State::Stopped, &config).await;

    // Restart on crash
    if crashed && config.server.wake_on_crash {
        warn!(target: "lazymc", "Server crashed, restarting...");
        Server::start(config, state, None).await;
    }

    Ok(())
}

/// Stop server through RCON.
#[cfg(feature = "rcon")]
async fn stop_server_rcon(config: &Config, server: &Server) -> bool {
    use crate::mc::rcon::Rcon;

    // RCON must be enabled
    if !config.rcon.enabled {
        trace!(target: "lazymc", "Not using RCON to stop server, disabled in config");
        return false;
    }

    // Grab RCON lock
    let rcon_lock = server.rcon_lock.acquire().await.unwrap();

    // Ensure RCON has cooled down
    let rcon_cooled_down = server
        .rcon_last_stop
        .lock()
        .await
        .map(|t| t.elapsed() >= RCON_COOLDOWN)
        .unwrap_or(true);
    if !rcon_cooled_down {
        debug!(target: "lazymc", "Not using RCON to stop server, in cooldown, used too recently");
        return false;
    }

    // Create RCON client
    let mut rcon = match Rcon::connect_config(config).await {
        Ok(rcon) => rcon,
        Err(err) => {
            error!(target: "lazymc", "Failed to RCON server to sleep: {}", err);
            return false;
        }
    };

    // Invoke stop
    if let Err(err) = rcon.cmd("stop").await {
        error!(target: "lazymc", "Failed to invoke stop through RCON: {}", err);
        return false;
    }

    // Set server to stopping state, update last RCON time
    server.rcon_last_stop.lock().await.replace(Instant::now());
    server.update_state(State::Stopping, config).await;

    // Gracefully close connection
    rcon.close().await;

    drop(rcon_lock);

    true
}

/// Stop server by sending SIGTERM signal.
///
/// Only available on Unix.
#[cfg(unix)]
async fn stop_server_signal(config: &Config, server: &Server) -> bool {
    // Grab PID
    let pid = match *server.pid.lock().await {
        Some(pid) => pid,
        None => {
            debug!(target: "lazymc", "Could not send stop signal to server process, PID unknown");
            return false;
        }
    };

    if !crate::os::kill_gracefully(pid) {
        error!(target: "lazymc", "Failed to send stop signal to server process");
        return false;
    }

    server
        .update_state_from(Some(State::Starting), State::Stopping, config)
        .await;
    server
        .update_state_from(Some(State::Started), State::Stopping, config)
        .await;

    true
}

/// Freeze server by sending SIGSTOP signal.
///
/// Only available on Unix.
#[cfg(unix)]
async fn freeze_server_signal(config: &Config, server: &Server) -> bool {
    // Grab PID
    let pid = match *server.pid.lock().await {
        Some(pid) => pid,
        None => {
            debug!(target: "lazymc", "Could not send freeze signal to server process, PID unknown");
            return false;
        }
    };

    if !os::freeze(pid) {
        error!(target: "lazymc", "Failed to send freeze signal to server process.");
    }

    server
        .update_state_from(Some(State::Starting), State::Stopped, config)
        .await;
    server
        .update_state_from(Some(State::Started), State::Stopped, config)
        .await;

    true
}

/// Unfreeze server by sending SIGCONT signal.
///
/// Only available on Unix.
#[cfg(unix)]
async fn unfreeze_server_signal(config: &Config, server: &Server) -> bool {
    // Grab PID
    let pid = match *server.pid.lock().await {
        Some(pid) => pid,
        None => {
            debug!(target: "lazymc", "Could not send unfreeze signal to server process, PID unknown");
            return false;
        }
    };

    if !os::unfreeze(pid) {
        error!(target: "lazymc", "Failed to send unfreeze signal to server process.");
    }

    server
        .update_state_from(Some(State::Stopping), State::Starting, config)
        .await;
    server
        .update_state_from(Some(State::Stopped), State::Starting, config)
        .await;

    true
}
