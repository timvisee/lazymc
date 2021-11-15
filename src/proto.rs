use std::io::prelude::*;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use bytes::BytesMut;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use tokio::io;
use tokio::io::AsyncReadExt;
use tokio::net::tcp::ReadHalf;

use crate::types;

/// Default minecraft protocol version name.
///
/// Just something to default to when real server version isn't known or when no hint is specified
/// in the configuration.
///
/// Should be kept up-to-date with latest supported Minecraft version by lazymc.
pub const PROTO_DEFAULT_VERSION: &str = "1.17.1";

/// Default minecraft protocol version.
///
/// Just something to default to when real server version isn't known or when no hint is specified
/// in the configuration.
///
/// Should be kept up-to-date with latest supported Minecraft version by lazymc.
pub const PROTO_DEFAULT_PROTOCOL: u32 = 756;

/// Compression threshold to use.
// TODO: read this from server.properties instead
pub const COMPRESSION_THRESHOLD: i32 = 256;

/// Default buffer size when reading packets.
const BUF_SIZE: usize = 8 * 1024;

/// Minecraft protocol packet IDs.
#[allow(unused)]
pub mod packets {
    pub mod handshake {
        pub const SERVER_HANDSHAKE: i32 = 0;
    }

    pub mod status {
        pub const CLIENT_STATUS: i32 = 0;
        pub const CLIENT_PING: i32 = 1;
        pub const SERVER_STATUS: i32 = 0;
        pub const SERVER_PING: i32 = 1;
    }

    pub mod login {
        pub const CLIENT_DISCONNECT: i32 = 0x00;
        pub const CLIENT_LOGIN_SUCCESS: i32 = 0x02;
        pub const CLIENT_SET_COMPRESSION: i32 = 0x03;
        pub const SERVER_LOGIN_START: i32 = 0x00;
    }

    pub mod play {
        pub const CLIENT_CHAT_MSG: i32 = 0x0F;
        pub const CLIENT_PLUGIN_MESSAGE: i32 = 0x18;
        pub const CLIENT_NAMED_SOUND_EFFECT: i32 = 0x19;
        pub const CLIENT_DISCONNECT: i32 = 0x1A;
        pub const CLIENT_KEEP_ALIVE: i32 = 0x21;
        pub const CLIENT_JOIN_GAME: i32 = 0x26;
        pub const CLIENT_PLAYER_POS_LOOK: i32 = 0x38;
        pub const CLIENT_RESPAWN: i32 = 0x3D;
        pub const CLIENT_SPAWN_POS: i32 = 0x4B;
        pub const CLIENT_SET_TITLE_SUBTITLE: i32 = 0x57;
        pub const CLIENT_TIME_UPDATE: i32 = 0x58;
        pub const CLIENT_SET_TITLE_TEXT: i32 = 0x59;
        pub const CLIENT_SET_TITLE_TIMES: i32 = 0x5A;
        pub const SERVER_CLIENT_SETTINGS: i32 = 0x05;
        pub const SERVER_PLUGIN_MESSAGE: i32 = 0x0A;
        pub const SERVER_PLAYER_POS: i32 = 0x11;
        pub const SERVER_PLAYER_POS_ROT: i32 = 0x12;
    }
}

/// Client state.
///
/// Note: this does not keep track of encryption states.
#[derive(Debug)]
pub struct Client {
    /// Current client state.
    pub state: Mutex<ClientState>,

    /// Compression state.
    ///
    /// 0 or positive if enabled, negative if disabled.
    pub compression: AtomicI32,
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

    /// Get compression threshold.
    pub fn compressed(&self) -> i32 {
        self.compression.load(Ordering::Relaxed)
    }

    /// Whether compression is used.
    pub fn is_compressed(&self) -> bool {
        self.compressed() >= 0
    }

    /// Set compression value.
    pub fn set_compression(&self, threshold: i32) {
        trace!(target: "lazymc", "Client now uses compression threshold of {}", threshold);
        self.compression.store(threshold, Ordering::Relaxed);
    }
}

impl Default for Client {
    fn default() -> Self {
        Self {
            state: Default::default(),
            compression: AtomicI32::new(-1),
        }
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

/// Client info, useful during connection handling.
#[derive(Debug, Default)]
pub struct ClientInfo {
    /// Client protocol version.
    pub protocol_version: Option<i32>,

    /// Client username.
    pub username: Option<String>,
}

impl ClientInfo {
    pub fn empty() -> Self {
        Self::default()
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

    /// Read packet ID from buffer, use remaining buffer as data.
    fn read_packet_id_data(mut buf: &[u8]) -> Result<Self, ()> {
        // Read packet ID, select buf
        let (read, packet_id) = types::read_var_int(buf)?;
        buf = &buf[read..];

        Ok(Self::new(packet_id, buf.to_vec()))
    }

    /// Decode packet from raw buffer.
    ///
    /// This decodes both compressed and uncompressed packets based on the client threshold
    /// preference.
    pub fn decode(client: &Client, mut buf: &[u8]) -> Result<Self, ()> {
        // Read length
        let (read, len) = types::read_var_int(buf)?;
        buf = &buf[read..][..len as usize];

        // If no compression is used, read remaining packet ID and data
        if !client.is_compressed() {
            // Read packet ID and data
            return Self::read_packet_id_data(buf);
        }

        // Read data length
        let (read, data_len) = types::read_var_int(buf)?;
        buf = &buf[read..];

        // If data length is zero, the rest is not compressed
        if data_len == 0 {
            return Self::read_packet_id_data(buf);
        }

        // Decompress packet ID and data section
        let mut decompressed = Vec::with_capacity(data_len as usize);
        ZlibDecoder::new(buf)
            .read_to_end(&mut decompressed)
            .map_err(|err| {
                error!(target: "lazymc", "Packet decompression error: {}", err);
            })?;

        // Decompressed data must match length
        if decompressed.len() != data_len as usize {
            error!(target: "lazymc", "Decompressed packet has different length than expected ({}b != {}b)", decompressed.len(), data_len);
            return Err(());
        }

        // Read decompressed packet ID
        return Self::read_packet_id_data(&decompressed);
    }

    /// Encode packet to raw buffer.
    ///
    /// This compresses packets based on the client threshold preference.
    pub fn encode(&self, client: &Client) -> Result<Vec<u8>, ()> {
        let threshold = client.compressed();
        if threshold >= 0 {
            self.encode_compressed(threshold)
        } else {
            self.encode_uncompressed()
        }
    }

    /// Encode compressed packet to raw buffer.
    fn encode_compressed(&self, threshold: i32) -> Result<Vec<u8>, ()> {
        // Packet payload: packet ID and data buffer
        let mut payload = types::encode_var_int(self.id)?;
        payload.extend_from_slice(&self.data);

        // Determine whether to compress, encode data length bytes
        let data_len = payload.len() as i32;
        let compress = data_len > threshold;
        let mut data_len_bytes =
            types::encode_var_int(if compress { data_len } else { 0 }).unwrap();

        // Compress payload
        if compress {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&payload).map_err(|err| {
                error!(target: "lazymc", "Failed to compress packet: {}", err);
            })?;
            payload = encoder.finish().map_err(|err| {
                error!(target: "lazymc", "Failed to compress packet: {}", err);
            })?;
        }

        // Encapsulate payload with packet and data length
        let len = data_len_bytes.len() as i32 + payload.len() as i32;
        let mut packet = types::encode_var_int(len)?;
        packet.append(&mut data_len_bytes);
        packet.append(&mut payload);

        Ok(packet)
    }

    /// Encode uncompressed packet to raw buffer.
    fn encode_uncompressed(&self) -> Result<Vec<u8>, ()> {
        let mut data = types::encode_var_int(self.id)?;
        data.extend_from_slice(&self.data);

        let len = data.len() as i32;
        let mut packet = types::encode_var_int(len)?;
        packet.append(&mut data);

        Ok(packet)
    }
}

/// Read raw packet from stream.
pub async fn read_packet(
    client: &Client,
    buf: &mut BytesMut,
    stream: &mut ReadHalf<'_>,
) -> Result<Option<(RawPacket, Vec<u8>)>, ()> {
    // Keep reading until we have at least 2 bytes
    while buf.len() < 2 {
        // Read packet from socket
        let mut tmp = Vec::with_capacity(BUF_SIZE);
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
        let mut tmp = Vec::with_capacity(BUF_SIZE);
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

    // Parse packet, use full buffer since we'll read the packet length again
    let raw = buf.split_to(consumed + len as usize);
    let packet = RawPacket::decode(client, &raw)?;

    Ok(Some((packet, raw.to_vec())))
}
