//! Minecraft protocol packet IDs.

#![allow(unused)]

pub mod handshake {
    pub const SERVER_HANDSHAKE: u8 = 0x00;
}

pub mod status {
    pub const CLIENT_STATUS: u8 = 0x0;
    pub const CLIENT_PING: u8 = 0x01;
    pub const SERVER_STATUS: u8 = 0x00;
    pub const SERVER_PING: u8 = 0x01;
}

pub mod login {
    pub const CLIENT_DISCONNECT: u8 = 0x00;
    pub const CLIENT_LOGIN_SUCCESS: u8 = 0x02;
    pub const CLIENT_SET_COMPRESSION: u8 = 0x03;
    pub const SERVER_LOGIN_START: u8 = 0x00;
}

pub mod play {
    pub const CLIENT_CHAT_MSG: u8 = 0x0F;
    pub const CLIENT_PLUGIN_MESSAGE: u8 = 0x18;
    pub const CLIENT_NAMED_SOUND_EFFECT: u8 = 0x19;
    pub const CLIENT_DISCONNECT: u8 = 0x1A;
    pub const CLIENT_KEEP_ALIVE: u8 = 0x21;
    pub const CLIENT_JOIN_GAME: u8 = 0x26;
    pub const CLIENT_PLAYER_POS_LOOK: u8 = 0x38;
    pub const CLIENT_RESPAWN: u8 = 0x3D;
    pub const CLIENT_SPAWN_POS: u8 = 0x4B;
    pub const CLIENT_SET_TITLE_SUBTITLE: u8 = 0x57;
    pub const CLIENT_TIME_UPDATE: u8 = 0x58;
    pub const CLIENT_SET_TITLE_TEXT: u8 = 0x59;
    pub const CLIENT_SET_TITLE_TIMES: u8 = 0x5A;
    pub const SERVER_CLIENT_SETTINGS: u8 = 0x05;
    pub const SERVER_PLUGIN_MESSAGE: u8 = 0x0A;
    pub const SERVER_PLAYER_POS: u8 = 0x11;
    pub const SERVER_PLAYER_POS_ROT: u8 = 0x12;
}
