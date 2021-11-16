//! Minecraft protocol packet IDs.

#![allow(unused)]

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
