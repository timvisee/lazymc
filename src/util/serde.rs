use std::net::{SocketAddr, ToSocketAddrs};

use serde::de::{Error, Unexpected};
use serde::{Deserialize, Deserializer};

fn str_to_maybe_addr<'de, D: Deserializer<'de>>(addr: &str) -> Result<SocketAddr, D::Error> {
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
        Error::invalid_value(Unexpected::Str(addr), &"IP or resolvable host and port")
    })
}

/// Deserialize a `Vec` into a `HashMap` by key.
pub fn to_socket_addrs<'de, D>(d: D) -> Result<SocketAddr, D::Error>
where
    D: Deserializer<'de>,
{
    // Deserialize string
    let addr = String::deserialize(d)?;

    str_to_maybe_addr::<D>(&addr)
}

/// Deserialize a string or an array of strings into one or many socket addresses
pub fn to_socket_addrs_multiple<'de, D>(d: D) -> Result<Box<[SocketAddr]>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<String>;
        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error, {
            Ok(vec![v.to_owned()])
        }
        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: Error, {
            Ok(vec![v])
        }
        fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>, {
            Vec::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
        }
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            std::write!(formatter, "a String or an array of Strings holding socket addresses")
        }
    }
    let strings = d.deserialize_any(Visitor)?;
    let mut addrs = Vec::with_capacity(strings.len());

    for s in strings {
        addrs.push(str_to_maybe_addr::<D>(&s)?);
    }

    Ok(addrs.into_boxed_slice())
}
