use crate::types;

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
    pub fn decode(mut buf: &mut [u8]) -> Result<Self, ()> {
        // Read length
        let (read, len) = types::read_var_int(buf)?;
        buf = &mut buf[read..][..len as usize];

        // Read packet ID, select buf
        let (read, packet_id) = types::read_var_int(buf)?;
        buf = &mut buf[read..];

        Ok(Self::new(packet_id, buf.to_vec()))
    }

    /// Encode packet to raw buffer.
    pub fn encode(&self) -> Result<Vec<u8>, ()> {
        let mut data = types::encode_var_int(self.id)?;
        data.extend_from_slice(&self.data);

        let len = data.len() as i32;
        let mut packet = types::encode_var_int(len)?;
        packet.append(&mut data);

        return Ok(packet);
    }
}
