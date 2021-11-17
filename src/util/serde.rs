use std::net::{SocketAddr, ToSocketAddrs};

use serde::de::{Error, Unexpected};
use serde::{Deserialize, Deserializer};

/// Deserialize a `Vec` into a `HashMap` by key.
pub fn to_socket_addrs<'de, D>(d: D) -> Result<SocketAddr, D::Error>
where
    D: Deserializer<'de>,
{
    // Deserialize string
    let addr = String::deserialize(d)?;

    // Try to socket address to resolve
    match addr.to_socket_addrs() {
        Ok(mut addr) => {
            if let Some(addr) = addr.next() {
                return Ok(addr);
            }
        }
        Err(err) => {
            dbg!(err);
        }
    }

    // Parse raw IP address
    addr.parse().map_err(|_| {
        Error::invalid_value(Unexpected::Str(&addr), &"IP or resolvable host and port")
    })
}
