use md5::{Digest, Md5};
use uuid::Uuid;

/// Offline player namespace.
const OFFLINE_PLAYER_NAMESPACE: &str = "OfflinePlayer:";

/// Get UUID for given player username.
fn player_uuid(username: &str) -> Uuid {
    java_name_uuid_from_bytes(username.as_bytes())
}

/// Get UUID for given offline player username.
pub fn offline_player_uuid(username: &str) -> Uuid {
    player_uuid(&format!("{OFFLINE_PLAYER_NAMESPACE}{username}"))
}

/// Java's `UUID.nameUUIDFromBytes`
///
/// Static factory to retrieve a type 3 (name based) `Uuid` based on the specified byte array.
///
/// Ported from: <https://git.io/J1b6A>
fn java_name_uuid_from_bytes(data: &[u8]) -> Uuid {
    let mut hasher = Md5::new();
    hasher.update(data);
    let mut md5: [u8; 16] = hasher.finalize().into();

    md5[6] &= 0x0f; // clear version
    md5[6] |= 0x30; // set to version 3
    md5[8] &= 0x3f; // clear variant
    md5[8] |= 0x80; // set to IETF variant

    Uuid::from_bytes(md5)
}
