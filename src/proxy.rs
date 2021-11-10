use std::error::Error;
use std::net::SocketAddr;

use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

/// Proxy the inbound stream to a target address.
pub async fn proxy(inbound: TcpStream, addr_target: SocketAddr) -> Result<(), Box<dyn Error>> {
    proxy_with_queue(inbound, addr_target, &[]).await
}

/// Proxy the inbound stream to a target address.
///
/// Send the queue to the target server before proxying.
pub async fn proxy_with_queue(
    mut inbound: TcpStream,
    addr_target: SocketAddr,
    queue: &[u8],
) -> Result<(), Box<dyn Error>> {
    // Set up connection to server
    // TODO: on connect fail, ping server and redirect to serve_status if offline
    let mut outbound = TcpStream::connect(addr_target).await?;

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    // Forward queued bytes to server once writable
    if !queue.is_empty() {
        wo.writable().await?;
        trace!(target: "lazymc", "Relaying {} queued bytes to server", queue.len());
        wo.write_all(&queue).await?;
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

    Ok(())
}
