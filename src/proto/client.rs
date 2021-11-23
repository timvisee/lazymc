use std::net::SocketAddr;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use minecraft_protocol::version::v1_14_4::handshake::Handshake;

/// Client state.
///
/// Note: this does not keep track of encryption states.
#[derive(Debug)]
pub struct Client {
    /// Client peer address.
    pub peer: SocketAddr,

    /// Current client state.
    pub state: Mutex<ClientState>,

    /// Compression state.
    ///
    /// 0 or positive if enabled, negative if disabled.
    pub compression: AtomicI32,
}

impl Client {
    /// Construct new client with given peer address.
    pub fn new(peer: SocketAddr) -> Self {
        Self {
            peer,
            state: Default::default(),
            compression: AtomicI32::new(-1),
        }
    }

    /// Construct dummy client.
    pub fn dummy() -> Self {
        Self::new("0.0.0.0:0".parse().unwrap())
    }

    /// Get client state.
    pub fn state(&self) -> ClientState {
        *self.state.lock().unwrap()
    }

    /// Set client state.
    pub fn set_state(&self, state: ClientState) {
        *self.state.lock().unwrap() = state;
    }

    /// Get compression threshold.
    pub fn compressed(&self) -> i32 {
        self.compression.load(Ordering::Relaxed)
    }

    /// Whether compression is used.
    pub fn is_compressed(&self) -> bool {
        self.compressed() >= 0
    }

    /// Set compression value.
    #[allow(unused)]
    pub fn set_compression(&self, threshold: i32) {
        trace!(target: "lazymc", "Client now uses compression threshold of {}", threshold);
        self.compression.store(threshold, Ordering::Relaxed);
    }
}

/// Protocol state a client may be in.
///
/// Note: this does not include the `play` state, because this is never used anymore when a client
/// reaches this state.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ClientState {
    /// Initial client state.
    Handshake,

    /// State to query server status.
    Status,

    /// State to login to server.
    Login,

    /// State to play on the server.
    #[allow(unused)]
    Play,
}

impl ClientState {
    /// From state ID.
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::Handshake),
            1 => Some(Self::Status),
            2 => Some(Self::Login),
            _ => None,
        }
    }

    /// Get state ID.
    pub fn to_id(self) -> i32 {
        match self {
            Self::Handshake => 0,
            Self::Status => 1,
            Self::Login => 2,
            Self::Play => -1,
        }
    }
}

impl Default for ClientState {
    fn default() -> Self {
        Self::Handshake
    }
}

/// Client info, useful during connection handling.
#[derive(Debug, Clone, Default)]
pub struct ClientInfo {
    /// Used protocol version.
    pub protocol: Option<u32>,

    /// Handshake as received from client.
    pub handshake: Option<Handshake>,

    /// Client username.
    pub username: Option<String>,
}

impl ClientInfo {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Get protocol version.
    pub fn protocol(&self) -> Option<u32> {
        self.protocol
            .or_else(|| self.handshake.as_ref().map(|h| h.protocol_version as u32))
    }
}
