//! Minecraft protocol packet IDs.

pub mod play;

pub mod handshake {
    use minecraft_protocol::version::v1_14_4::handshake::*;

    pub const SERVER_HANDSHAKE: u8 = Handshake::PACKET_ID;
}

pub mod status {
    use minecraft_protocol::version::v1_14_4::status::*;

    pub const CLIENT_STATUS: u8 = StatusResponse::PACKET_ID;
    pub const CLIENT_PING: u8 = PingResponse::PACKET_ID;
    pub const SERVER_STATUS: u8 = StatusRequest::PACKET_ID;
    pub const SERVER_PING: u8 = PingRequest::PACKET_ID;
}

pub mod login {
    use minecraft_protocol::version::v1_14_4::login::*;

    #[cfg(feature = "lobby")]
    pub const CLIENT_DISCONNECT: u8 = LoginDisconnect::PACKET_ID;
    pub const CLIENT_LOGIN_SUCCESS: u8 = LoginSuccess::PACKET_ID;
    pub const CLIENT_SET_COMPRESSION: u8 = SetCompression::PACKET_ID;
    #[cfg(feature = "lobby")]
    pub const CLIENT_ENCRYPTION_REQUEST: u8 = EncryptionRequest::PACKET_ID;
    pub const CLIENT_LOGIN_PLUGIN_REQUEST: u8 = LoginPluginRequest::PACKET_ID;
    pub const SERVER_LOGIN_START: u8 = LoginStart::PACKET_ID;
    #[cfg(feature = "lobby")]
    pub const SERVER_LOGIN_PLUGIN_RESPONSE: u8 = LoginPluginResponse::PACKET_ID;
}
