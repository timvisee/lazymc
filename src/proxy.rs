use std::error::Error;
use std::net::SocketAddr;

use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::net;

/// Proxy the inbound stream to a target address.
pub async fn proxy(inbound: TcpStream, addr_target: SocketAddr) -> Result<(), Box<dyn Error>> {
    proxy_with_queue(inbound, addr_target, &[]).await
}

/// Proxy the inbound stream to a target address.
///
/// Send the queue to the target server before proxying.
pub async fn proxy_with_queue(
    inbound: TcpStream,
    addr_target: SocketAddr,
    queue: &[u8],
) -> Result<(), Box<dyn Error>> {
    // Set up connection to server
    // TODO: on connect fail, ping server and redirect to serve_status if offline
    let outbound = TcpStream::connect(addr_target).await?;

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
