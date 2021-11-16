use std::io::prelude::*;

use bytes::BytesMut;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use tokio::io;
use tokio::io::AsyncReadExt;
use tokio::net::tcp::ReadHalf;

use crate::proto::client::Client;
use crate::proto::BUF_SIZE;
use crate::types;

/// Raw Minecraft packet.
///
/// Having a packet ID and a raw data byte array.
pub struct RawPacket {
    /// Packet ID.
    pub id: u8,

    /// Packet data.
    pub data: Vec<u8>,
}

impl RawPacket {
    /// Construct new raw packet.
    pub fn new(id: u8, data: Vec<u8>) -> Self {
        Self { id, data }
    }

    /// Read packet ID from buffer, use remaining buffer as data.
    fn read_packet_id_data(mut buf: &[u8]) -> Result<Self, ()> {
        // Read packet ID, select buf
        let (read, packet_id) = types::read_var_int(buf)?;
        buf = &buf[read..];

        Ok(Self::new(packet_id as u8, buf.to_vec()))
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
        Self::read_packet_id_data(&decompressed)
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
        let mut payload = types::encode_var_int(self.id as i32)?;
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
        let mut data = types::encode_var_int(self.id as i32)?;
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
