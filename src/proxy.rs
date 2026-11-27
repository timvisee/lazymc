use std::error::Error;
use std::net::{SocketAddr, SocketAddrV6};
use std::time::Duration;

use bytes::BytesMut;
use proxy_protocol::version2::{ProxyAddresses, ProxyCommand, ProxyTransportProtocol};
use proxy_protocol::EncodeError;
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::net;

/// Proxy the inbound stream to a target address.
pub async fn proxy(
    inbound: TcpStream,
    proxy_header: ProxyHeader,
    addr_target: SocketAddr,
    client: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    proxy_with_queue(inbound, proxy_header, addr_target, &[], client).await
}

/// Proxy the inbound stream to a target address.
///
/// Send the queue to the target server before proxying.
pub async fn proxy_with_queue(
    inbound: TcpStream,
    proxy_header: ProxyHeader,
    addr_target: SocketAddr,
    queue: &[u8],
    client: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    // Set up connection to server
    // TODO: on connect fail, ping server and redirect to serve_status if offline
    let mut outbound = TcpStream::connect(addr_target).await?;

    // Add proxy header
    match proxy_header {
        ProxyHeader::None => {}
        ProxyHeader::Local => {
            let header = local_proxy_header()?;
            outbound.write_all(&header).await?;
        }
        ProxyHeader::Proxy => {
            let header = stream_proxy_header(&inbound, client)?;
            outbound.write_all(&header).await?;
        }
    }

    // Start proxy on both streams
    proxy_inbound_outbound_with_queue(inbound, outbound, &[], queue).await
}

/// Proxy the inbound stream to a target address.
///
/// Send the queue to the target server before proxying.
// TODO: find better name for this
pub async fn proxy_inbound_outbound_with_queue(
    mut inbound: TcpStream,
    mut outbound: TcpStream,
    inbound_queue: &[u8],
    outbound_queue: &[u8],
) -> Result<(), Box<dyn Error>> {
    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    // Forward queued bytes to client once writable
    if !inbound_queue.is_empty() {
        wi.writable().await?;
        trace!(target: "lazymc", "Relaying {} queued bytes to client", inbound_queue.len());
        wi.write_all(inbound_queue).await?;
    }

    // Forward queued bytes to server once writable
    if !outbound_queue.is_empty() {
        wo.writable().await?;
        trace!(target: "lazymc", "Relaying {} queued bytes to server", outbound_queue.len());
        wo.write_all(outbound_queue).await?;
    }

    let client_to_server = async {
        io::copy(&mut ri, &mut wo).await?;
        wo.shutdown().await
    };
    let server_to_client = async {
        io::copy(&mut ro, &mut wi).await?;
        wi.shutdown().await
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    // Gracefully close connection if not done already
    net::close_tcp_stream(inbound).await?;

    Ok(())
}

/// Proxy header.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ProxyHeader {
    /// Do not add proxy header.
    None,

    /// Header for locally initiated connection.
    #[allow(unused)]
    Local,

    /// Header for proxied connection.
    Proxy,
}

impl ProxyHeader {
    /// Changes to `None` if `false` if given.
    ///
    /// `None` stays `None`.
    pub fn not_none(self, not_none: bool) -> Self {
        if not_none {
            self
        } else {
            Self::None
        }
    }
}

/// Get the proxy header for a locally initiated connection.
///
/// This header may be sent over the outbound stream to signal client information.
pub fn local_proxy_header() -> Result<BytesMut, EncodeError> {
    // Build proxy header
    let header = proxy_protocol::ProxyHeader::Version2 {
        command: ProxyCommand::Local,
        transport_protocol: ProxyTransportProtocol::Stream,
        addresses: ProxyAddresses::Unspec,
    };

    proxy_protocol::encode(header)
}

/// Get the proxy header for the given inbound stream.
///
/// This header may be sent over the outbound stream to signal client information.
///
/// `client` is the client's address: the stream's peer, or the one a trusted proxy in front gave in its own
/// PROXY header (`public.proxy_address`). A v4/v6 mix is written as v6 (v4-mapped).
pub fn stream_proxy_header(
    inbound: &TcpStream,
    client: SocketAddr,
) -> Result<BytesMut, EncodeError> {
    let peer = client;
    let local = inbound
        .local_addr()
        .expect("Local address not known for TCP stream");
    let v6 = |a: SocketAddr| match a {
        SocketAddr::V4(v4) => {
            SocketAddr::V6(SocketAddrV6::new(v4.ip().to_ipv6_mapped(), v4.port(), 0, 0))
        }
        v6 => v6,
    };
    let (peer, local) = match (peer, local) {
        (SocketAddr::V4(_), SocketAddr::V6(_)) | (SocketAddr::V6(_), SocketAddr::V4(_)) => {
            (v6(peer), v6(local))
        }
        same => same,
    };

    // Build proxy header
    let header = proxy_protocol::ProxyHeader::Version2 {
        command: ProxyCommand::Proxy,
        transport_protocol: ProxyTransportProtocol::Stream,
        addresses: match (peer, local) {
            (SocketAddr::V4(source), SocketAddr::V4(destination)) => ProxyAddresses::Ipv4 {
                source,
                destination,
            },
            (SocketAddr::V6(source), SocketAddr::V6(destination)) => ProxyAddresses::Ipv6 {
                source,
                destination,
            },
            (_, _) => unreachable!(),
        },
    };

    proxy_protocol::encode(header)
}

/// How long a client on `public.proxy_address` gets to send its PROXY header.
const PROXY_HEADER_TIMEOUT: Duration = Duration::from_secs(5);

/// The PROXY protocol v2 signature (the first 12 bytes of every v2 header).
const PROXY_V2_SIGNATURE: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];

/// Reads the PROXY protocol v2 header a trusted proxy sends first on `public.proxy_address`, and returns the
/// client address it gives (`None` for a LOCAL header, the proxy's own health check: use the stream's peer). Exactly
/// the header is consumed, so the Minecraft bytes that follow are untouched. Anything else (no header, v1, a bad or
/// oversized one, too slow) is an error and the connection should be dropped.
pub async fn read_proxy_header(
    inbound: &mut TcpStream,
) -> Result<Option<SocketAddr>, Box<dyn Error>> {
    tokio::time::timeout(PROXY_HEADER_TIMEOUT, async {
        // 12-byte signature, version/command, family/protocol, 2-byte length of what follows.
        let mut head = [0u8; 16];
        inbound.read_exact(&mut head).await?;
        if head[..12] != PROXY_V2_SIGNATURE {
            return Err("no PROXY v2 header".into());
        }
        let len = u16::from_be_bytes([head[14], head[15]]) as usize;
        // Addresses (at most 36 bytes for IPv6) plus TLVs; anything near the limit is not a real proxy.
        if len > 512 {
            return Err("PROXY v2 header too long".into());
        }
        let mut buf = BytesMut::with_capacity(16 + len);
        buf.extend_from_slice(&head);
        buf.resize(16 + len, 0);
        inbound.read_exact(&mut buf[16..]).await?;
        let header =
            proxy_protocol::parse(&mut buf).map_err(|e| format!("bad PROXY v2 header: {e}"))?;
        match header {
            proxy_protocol::ProxyHeader::Version2 {
                command: ProxyCommand::Local,
                ..
            } => Ok(None),
            proxy_protocol::ProxyHeader::Version2 {
                command: ProxyCommand::Proxy,
                transport_protocol: ProxyTransportProtocol::Stream,
                addresses: ProxyAddresses::Ipv4 { source, .. },
            } => Ok(Some(SocketAddr::V4(source))),
            proxy_protocol::ProxyHeader::Version2 {
                command: ProxyCommand::Proxy,
                transport_protocol: ProxyTransportProtocol::Stream,
                addresses: ProxyAddresses::Ipv6 { source, .. },
            } => Ok(Some(SocketAddr::V6(source))),
            _ => Err("unsupported PROXY v2 header (not TCP over IPv4/IPv6)".into()),
        }
    })
    .await
    .map_err(|_| -> Box<dyn Error> { "no PROXY v2 header in time".into() })?
}

// The PROXY listener's header reader.
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use tokio::net::TcpListener;

    /// A connected pair: what a client writes on the first arrives on the second.
    async fn pair(sent: Vec<u8>) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut c = TcpStream::connect(addr).await.unwrap();
            c.write_all(&sent).await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        listener.accept().await.unwrap().0
    }

    fn header(command: ProxyCommand, source: SocketAddrV4) -> Vec<u8> {
        let addresses = match command {
            ProxyCommand::Local => ProxyAddresses::Unspec,
            ProxyCommand::Proxy => ProxyAddresses::Ipv4 {
                source,
                destination: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 25566),
            },
        };
        proxy_protocol::encode(proxy_protocol::ProxyHeader::Version2 {
            command,
            transport_protocol: ProxyTransportProtocol::Stream,
            addresses,
        })
        .unwrap()
        .to_vec()
    }

    #[tokio::test]
    async fn takes_the_client_from_the_header_and_leaves_the_rest() {
        let player = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 51234);
        let mut sent = header(ProxyCommand::Proxy, player);
        sent.extend_from_slice(b"\x10\x00minecraft handshake");
        let mut s = pair(sent).await;
        assert_eq!(
            read_proxy_header(&mut s).await.unwrap(),
            Some(SocketAddr::V4(player))
        );
        let mut rest = [0u8; 21];
        s.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"\x10\x00minecraft handshake");
    }

    #[tokio::test]
    async fn a_local_header_means_the_peer_itself() {
        let mut s = pair(header(
            ProxyCommand::Local,
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
        ))
        .await;
        assert_eq!(read_proxy_header(&mut s).await.unwrap(), None);
    }

    #[tokio::test]
    async fn no_header_a_v1_header_or_garbage_is_refused() {
        for sent in [
            b"\x10\x00\xfd\x05\x10play.example.com".to_vec(),
            b"PROXY TCP4 203.0.113.7 127.0.0.1 51234 25566\r\n".to_vec(),
            vec![0u8; 64],
        ] {
            let mut s = pair(sent).await;
            assert!(read_proxy_header(&mut s).await.is_err());
        }
    }

    #[tokio::test]
    async fn an_oversized_header_is_refused() {
        let mut sent = PROXY_V2_SIGNATURE.to_vec();
        sent.extend_from_slice(&[0x21, 0x11, 0xff, 0xff]);
        let mut s = pair(sent).await;
        assert!(read_proxy_header(&mut s).await.is_err());
    }

    #[tokio::test]
    async fn the_header_sent_on_names_the_given_client() {
        let player = SocketAddr::V6("[2001:db8::7]:51234".parse().unwrap());
        let s = pair(vec![]).await;
        let mut bytes = stream_proxy_header(&s, player).unwrap();
        match proxy_protocol::parse(&mut bytes).unwrap() {
            proxy_protocol::ProxyHeader::Version2 {
                addresses: ProxyAddresses::Ipv6 { source, .. },
                ..
            } => assert_eq!(SocketAddr::V6(source), player),
            other => panic!("unexpected {other:?}"),
        }
    }
}
