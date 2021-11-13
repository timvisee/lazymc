use std::sync::Mutex;

use bytes::BytesMut;
use tokio::io;
use tokio::io::AsyncReadExt;
use tokio::net::tcp::ReadHalf;

use crate::types;

/// Default minecraft protocol version name.
///
/// Send to clients when the server is sleeping when the real server version is not known yet.
pub const PROTO_DEFAULT_VERSION: &str = "1.17.1";

/// Default minecraft protocol version.
///
/// Send to clients when the server is sleeping when the real server version is not known yet, and
/// with server status polling requests.
pub const PROTO_DEFAULT_PROTOCOL: u32 = 756;

/// Handshake state, handshake packet ID.
pub const HANDSHAKE_PACKET_ID_HANDSHAKE: i32 = 0;

/// Status state, status packet ID.
pub const STATUS_PACKET_ID_STATUS: i32 = 0;

/// Status state, ping packet ID.
pub const STATUS_PACKET_ID_PING: i32 = 1;

/// Login state, login start packet ID.
pub const LOGIN_PACKET_ID_LOGIN_START: i32 = 0;

/// Login state, disconnect packet ID.
pub const LOGIN_PACKET_ID_DISCONNECT: i32 = 0;

/// Login state, login success packet ID.
pub const LOGIN_PACKET_ID_LOGIN_SUCCESS: i32 = 2;

pub mod packets {
    pub mod play {
        pub const CLIENT_JOIN_GAME: i32 = 0x26;
        pub const SERVER_CLIENT_SETTINGS: i32 = 0x05;
        pub const SERVER_PLUGIN_MESSAGE: i32 = 0x0A;
        pub const SERVER_PLAYER_POS_ROT: i32 = 0x12;
        pub const SERVER_PLAYER_POS: i32 = 0x11;
        pub const CLIENT_KEEP_ALIVE: i32 = 0x21;
        pub const CLIENT_PLAYER_POS_LOOK: i32 = 0x38;
        pub const CLIENT_RESPAWN: i32 = 0x3D;
        pub const CLIENT_SET_TITLE_TEXT: i32 = 0x59;
        pub const CLIENT_SET_TITLE_SUBTITLE: i32 = 0x57;
        pub const CLIENT_SET_TITLE_TIMES: i32 = 0x5A;
        pub const CLIENT_TIME_UPDATE: i32 = 0x58;
        pub const CLIENT_CHAT_MSG: i32 = 0x0F;
        pub const CLIENT_SPAWN_POS: i32 = 0x4B;
        pub const CLIENT_DISCONNECT: i32 = 0x1A;
    }
}

/// Client state.
///
/// Note: this does not keep track of compression/encryption states because packets are never
/// inspected when these modes are enabled.
#[derive(Debug, Default)]
pub struct Client {
    /// Current client state.
    pub state: Mutex<ClientState>,
}

impl Client {
    /// Get client state.
    pub fn state(&self) -> ClientState {
        *self.state.lock().unwrap()
    }

    /// Set client state.
    pub fn set_state(&self, state: ClientState) {
        *self.state.lock().unwrap() = state;
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

/// Raw Minecraft packet.
///
/// Having a packet ID and a raw data byte array.
pub struct RawPacket {
    /// Packet ID.
    pub id: i32,

    /// Packet data.
    pub data: Vec<u8>,
}

impl RawPacket {
    /// Construct new raw packet.
    pub fn new(id: i32, data: Vec<u8>) -> Self {
        Self { id, data }
    }

    /// Decode packet from raw buffer.
    pub fn decode(mut buf: &[u8]) -> Result<Self, ()> {
        // Read length
        let (read, len) = types::read_var_int(buf)?;
        buf = &buf[read..][..len as usize];

        // Read packet ID, select buf
        let (read, packet_id) = types::read_var_int(buf)?;
        buf = &buf[read..];

        Ok(Self::new(packet_id, buf.to_vec()))
    }

    /// Encode packet to raw buffer.
    pub fn encode(&self) -> Result<Vec<u8>, ()> {
        let mut data = types::encode_var_int(self.id)?;
        data.extend_from_slice(&self.data);

        let len = data.len() as i32;
        let mut packet = types::encode_var_int(len)?;
        packet.append(&mut data);

        Ok(packet)
    }
}

/// Read raw packet from stream.
///
/// Note: this does not support reading compressed/encrypted packets.
/// We should never need this though, as we're done reading user packets before any of this is
/// enabled. See: https://wiki.vg/Protocol#Packet_format
pub async fn read_packet(
    buf: &mut BytesMut,
    stream: &mut ReadHalf<'_>,
) -> Result<Option<(RawPacket, Vec<u8>)>, ()> {
    // Keep reading until we have at least 2 bytes
    while buf.len() < 2 {
        // Read packet from socket
        let mut tmp = Vec::with_capacity(64);
        match stream.read_buf(&mut tmp).await {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::ConnectionReset => return Ok(None),
            Err(err) => {
                dbg!(err);
                return Err(());
            }
        }

        if tmp.is_empty() {
            return Ok(None);
        }
        buf.extend(tmp);
    }

    // Attempt to read packet length
    let (consumed, len) = match types::read_var_int(buf) {
        Ok(result) => result,
        Err(err) => {
            error!(target: "lazymc", "Malformed packet, could not read packet length");
            return Err(err);
        }
    };

    // Keep reading until we have all packet bytes
    while buf.len() < consumed + len as usize {
        // Read packet from socket
        let mut tmp = Vec::with_capacity(64);
        match stream.read_buf(&mut tmp).await {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::ConnectionReset => return Ok(None),
            Err(err) => {
                dbg!(err);
                return Err(());
            }
        }

        if tmp.is_empty() {
            return Ok(None);
        }

        buf.extend(tmp);
    }

    // Parse packet
    let raw = buf.split_to(consumed + len as usize);
    let packet = RawPacket::decode(&raw)?;

    Ok(Some((packet, raw.to_vec())))
}
