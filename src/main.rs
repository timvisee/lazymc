#![allow(unused)]

pub mod config;
pub mod protocol;
pub mod types;

use std::error::Error;

use bytes::BytesMut;
use futures::future::poll_fn;
use futures::FutureExt;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::status::{PingRequest, PingResponse};
use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::io::ReadBuf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::unbounded_channel;

use config::*;
use protocol::RawPacket;

#[tokio::main]
async fn main() -> Result<(), ()> {
    println!("Public address: {}", ADDRESS_PUBLIC);
    println!("Proxy address: {}", ADDRESS_PROXY);

    // Listen for new connections
    // TODO: do not drop error here
    let listener = TcpListener::bind(ADDRESS_PUBLIC).await.map_err(|_| ())?;

    // Proxy all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        let transfer = proxy(inbound, ADDRESS_PROXY.to_string()).map(|r| {
            if let Err(e) = r {
                println!("Failed to proxy: {:?}", e);
            }
        });

        tokio::spawn(transfer);
    }

    Ok(())
}

/// Proxy the given inbound stream to a target address.
// TODO: do not drop error here, return Box<dyn Error>
async fn proxy(mut inbound: TcpStream, addr_target: String) -> Result<(), ()> {
    // TODO: do not drop error here
    let mut outbound = TcpStream::connect(addr_target).await.map_err(|_| ())?;

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    let (client_send_queue, mut client_to_send) = unbounded_channel::<Vec<u8>>();

    let client_to_server = async {
        // Wait for readable state
        while ri.readable().await.is_ok() {
            // Poll until we have data available
            let mut poll_buf = [0; 10];
            let mut poll_buf = ReadBuf::new(&mut poll_buf);
            // TODO: do not drop error here!
            let read = poll_fn(|cx| ri.poll_peek(cx, &mut poll_buf))
                .await
                .map_err(|_| ())?;
            if read == 0 {
                continue;
            }

            // TODO: remove
            // eprintln!("READ {}", read);

            // Read packet from socket
            let mut buf = Vec::with_capacity(64);
            // TODO: do not drop error here
            let read = ri.try_read_buf(&mut buf).map_err(|_| ())?;
            if read == 0 {
                continue;
            }

            // PING PACKET TEST
            eprintln!("PACKET {:?}", buf.as_slice());

            match RawPacket::decode(buf.as_mut_slice()) {
                Ok(packet) => {
                    eprintln!("PACKET ID: {}", packet.id);
                    eprintln!("PACKET DATA: {:?}", packet.data);

                    if packet.id == 0 {
                        // Catch status packet
                        eprintln!("PACKET STATUS");

                        use minecraft_protocol::data::chat::{Message, Payload};
                        use minecraft_protocol::data::server_status::*;
                        use minecraft_protocol::version::v1_14_4::status::*;

                        // Build status response
                        let server_status = ServerStatus {
                            version: ServerVersion {
                                name: String::from("1.16.5"),
                                protocol: 754,
                            },
                            description: Message::new(Payload::text(LABEL_SERVER_SLEEPING)),
                            players: OnlinePlayers {
                                online: 0,
                                max: 0,
                                sample: vec![],
                            },
                        };

                        let status_response = StatusResponse { server_status };

                        let mut vec = Vec::new();
                        status_response.encode(&mut vec).unwrap();

                        let status_packet = RawPacket::new(0, vec);
                        let response = status_packet.encode()?;

                        client_send_queue
                            .send(response)
                            .expect("failed to queue status response");

                        continue;
                    }

                    if packet.id == 1 {
                        // Catch ping packet
                        if let Ok(ping) = PingRequest::decode(&mut packet.data.as_slice()) {
                            eprintln!("PACKET PING: {}", ping.time);

                            let response = packet.encode()?;
                            client_send_queue
                                .send(response)
                                .expect("failed to queue ping response");

                            continue;
                        } else {
                            eprintln!("PACKET PING PARSE ERROR!");
                        }
                    }
                }
                Err(()) => eprintln!("ERROR PARSING PACKET"),
            }

            // Forward data to server
            wo.write_all(&buf).await.expect("failed to write to server");

            // io::copy(&mut ri, &mut wo).await?;
        }

        // io::copy(&mut ri, &mut wo).await?;

        // TODO: do not drop error here
        wo.shutdown().await.map_err(|_| ())
    };

    let server_to_client = async {
        // let proxy = io::copy(&mut ro, &mut wi);

        // Server packts to send to client, add to client sending queue
        let proxy = async {
            // Wait for readable state
            while ro.readable().await.is_ok() {
                // Poll until we have data available
                let mut poll_buf = [0; 10];
                let mut poll_buf = ReadBuf::new(&mut poll_buf);
                // TODO: do not drop error here
                let read = poll_fn(|cx| ro.poll_peek(cx, &mut poll_buf))
                    .await
                    .map_err(|_| ())?;
                if read == 0 {
                    continue;
                }

                // TODO: remove
                // eprintln!("READ {}", read);

                // Read packet from socket
                let mut buf = Vec::with_capacity(64);
                // TODO: do not drop error here
                let read = ro.try_read_buf(&mut buf).map_err(|_| ())?;
                if read == 0 {
                    continue;
                }

                assert_eq!(buf.len(), read);

                client_send_queue.send(buf);

                // Forward data to server
                // TODO: do not drop error here
                // wo.write_all(&buf).await.map_err(|_| ())?;

                // io::copy(&mut ri, &mut wo).await?;
            }

            Ok(())
        };

        // Push client sending queue to client
        let other = async {
            loop {
                let msg = poll_fn(|cx| client_to_send.poll_recv(cx))
                    .await
                    .expect("failed to poll_fn");

                wi.write_all(msg.as_ref())
                    .await
                    .expect("failed to write to client");
            }

            Ok(())
        };

        tokio::try_join!(proxy, other)?;

        // TODO: do not drop error here
        wi.shutdown().await.map_err(|_| ())
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}
