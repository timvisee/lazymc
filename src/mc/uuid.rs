use md5::{Digest, Md5};
use uuid::Uuid;

/// Offline player namespace.
const OFFLINE_PLAYER_NAMESPACE: &str = "OfflinePlayer:";

/// Get UUID for given player username.
pub fn player_uuid(username: &str) -> Uuid {
    Uuid::from_bytes(jdk_name_uuid_from_bytes(username.as_bytes()))
}

/// Get UUID for given offline player username.
pub fn offline_player_uuid(username: &str) -> Uuid {
    player_uuid(&format!("{}{}", OFFLINE_PLAYER_NAMESPACE, username))
}

/// Java's `UUID.nameUUIDFromBytes`.
///
/// Static factory to retrieve a type 3 (name based) `Uuid` based on the specified byte array.
///
/// Ported from: https://github.com/AdoptOpenJDK/openjdk-jdk8u/blob/9a91972c76ddda5c1ce28b50ca38cbd8a30b7a72/jdk/src/share/classes/java/util/UUID.java#L153-L175
fn jdk_name_uuid_from_bytes(data: &[u8]) -> [u8; 16] {
    let mut hasher = Md5::new();
    hasher.update(data);
    hasher.finalize().into()
}
