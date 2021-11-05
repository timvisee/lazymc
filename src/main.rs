use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use futures::FutureExt;
use std::error::Error;

/// Public address for users to connect to.
const ADDRESS_PUBLIC: &str = "127.0.0.1:9090";

/// Minecraft server address to proxy to.
const ADDRESS_PROXY: &str = "127.0.0.1:9091";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("Public address: {}", ADDRESS_PUBLIC);
    println!("Proxy address: {}", ADDRESS_PROXY);

    // Listen for new connections
    let listener = TcpListener::bind(ADDRESS_PUBLIC).await?;

    // Proxy all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        let transfer = proxy(inbound, ADDRESS_PROXY.to_string()).map(|r| {
            if let Err(e) = r {
                println!("Failed to proxy: {}", e);
            }
        });

        tokio::spawn(transfer);
    }

    Ok(())
}

/// Proxy the given inbound stream to a target address.
async fn proxy(mut inbound: TcpStream, addr_target: String) -> Result<(), Box<dyn Error>> {
    let mut outbound = TcpStream::connect(addr_target).await?;

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

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
