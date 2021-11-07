#![allow(unused)]

pub mod config;
pub mod protocol;
pub mod types;

use std::error::Error;

use bytes::BytesMut;
use futures::future::poll_fn;
use futures::FutureExt;
use futures::TryFutureExt;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::data::server_status::*;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::status::{PingRequest, PingResponse, StatusResponse};
use tokio::io;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::ReadBuf;
use tokio::net::tcp::ReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::unbounded_channel;

use config::*;
use protocol::{Client, ClientState, RawPacket};

#[tokio::main]
async fn main() -> Result<(), ()> {
    println!(
        "Proxying public {} to internal {}",
        ADDRESS_PUBLIC, ADDRESS_PROXY
    );

    // Listen for new connections
    // TODO: do not drop error here
    let listener = TcpListener::bind(ADDRESS_PUBLIC).await.map_err(|_| ())?;

    // Proxy all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        let client = Client::default();
        eprintln!("New client");

        let transfer = proxy(client, inbound, ADDRESS_PROXY.to_string()).map(|r| {
            if let Err(e) = r {
                println!("Failed to proxy: {:?}", e);
            }
        });

        tokio::spawn(transfer);
    }

    Ok(())
}

/// Read raw packet from stream.
async fn read_packet<'a>(
    buf: &mut BytesMut,
    stream: &mut ReadHalf<'a>,
) -> Result<Option<(RawPacket, Vec<u8>)>, ()> {
    // // Wait until socket is readable
    // if stream.readable().await.is_err() {
    //     eprintln!("Socket not readable!");
    //     return Ok(None);
    // }

    // Keep reading until we have at least 2 bytes
    while buf.len() < 2 {
        // Read packet from socket
        let mut tmp = Vec::with_capacity(64);
        stream.read_buf(&mut tmp).await;
        if tmp.is_empty() {
            return Ok(None);
        }
        buf.extend(tmp);
    }

    // Attempt to read packet length
    let (consumed, len) = match types::read_var_int(&buf) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Failed to read packet length, should retry!");
            eprintln!("{:?}", (&buf).as_ref());
            return Err(err);
        }
    };

    // Keep reading until we have all packet bytes
    while buf.len() < consumed + len as usize {
        // Read packet from socket
        let mut tmp = Vec::with_capacity(64);
        stream.read_buf(&mut tmp).await;
        if tmp.is_empty() {
            return Ok(None);
        }

        buf.extend(tmp);
    }

    // Parse packet
    let raw = buf.split_to(consumed + len as usize);
    let packet = RawPacket::decode(&raw)?;

    Ok(Some((packet, raw.to_vec())))
}

/// Proxy the given inbound stream to a target address.
// TODO: do not drop error here, return Box<dyn Error>
async fn proxy(mut client: Client, mut inbound: TcpStream, addr_target: String) -> Result<(), ()> {
    let mut outbound = TcpStream::connect(addr_target).await.map_err(|_| ())?;

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    let (client_send_queue, mut client_to_send) = unbounded_channel::<Vec<u8>>();

    let client_to_server = async {
        // Incoming buffer
        let mut buf = BytesMut::new();

        loop {
            // In login state, proxy raw data
            if client.state() == ClientState::Login {
                eprintln!("STARTED FULL PROXY");

                wo.writable().await;

                // Forward remaining buffer
                wo.write_all(&buf).await.map_err(|_| ())?;
                buf.clear();

                // Forward rest of data
                io::copy(&mut ri, &mut wo).await.map_err(|_| ())?;
                break;
            }

            // Read packet from stream
            let (packet, raw) = match read_packet(&mut buf, &mut ri).await {
                Ok(Some(packet)) => packet,
                Ok(None) => {
                    eprintln!("Closing connection, could not read more");
                    break;
                }
                Err(_) => {
                    // Forward raw packet to server
                    wo.write_all(&buf).await.expect("failed to write to server");
                    buf.clear();
                    continue;
                }
            };

            // Show packet details
            eprintln!("PACKET {:?}", raw.as_slice());
            eprintln!("PACKET ID: {}", packet.id);
            eprintln!("PACKET DATA: {:?}", packet.data);

            // Hijack handshake
            if client.state() == ClientState::Handshake
                && packet.id == protocol::STATUS_PACKET_ID_STATUS
            {
                if let Ok(handshake) = Handshake::decode(&mut packet.data.as_slice()) {
                    eprintln!("# PACKET HANDSHAKE");
                    eprintln!("SWITCHING CLIENT STATE: {}", handshake.next_state);

                    // TODO: do not panic here
                    client.set_state(
                        ClientState::from_id(handshake.next_state)
                            .expect("unknown next client state"),
                    );
                } else {
                    eprintln!("HANDSHAKE ERROR");
                }
            }

            // Hijack server status packet
            if client.state() == ClientState::Status
                && packet.id == protocol::STATUS_PACKET_ID_STATUS
            {
                eprintln!("# PACKET STATUS");

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
                let server_status = StatusResponse { server_status };

                let mut data = Vec::new();
                server_status.encode(&mut data).map_err(|_| ())?;

                let response = RawPacket::new(0, data).encode()?;
                client_send_queue
                    .send(response)
                    .expect("failed to queue status response");
                continue;
            }

            // Hijack ping packet
            if client.state() == ClientState::Status && packet.id == protocol::STATUS_PACKET_ID_PING
            {
                eprintln!("# PACKET PING");
                client_send_queue
                    .send(raw)
                    .expect("failed to queue ping response");
                continue;
            }

            // Forward raw packet to server
            wo.write_all(&raw).await.expect("failed to write to server");
        }

        wo.shutdown().await.map_err(|_| ())
    };

    let server_to_client = async {
        // Server packts to send to client, add to client sending queue
        let proxy = async {
            // Incoming buffer
            let mut buf = BytesMut::new();

            loop {
                // In login state, simply proxy all
                if client.state() == ClientState::Login {
                    // if true {
                    // if true {
                    eprintln!("STARTED FULL PROXY");

                    // // Wait until socket is readable
                    // if ro.readable().await.is_err() {
                    //     eprintln!("Socket not readable!");
                    //     break;
                    // }

                    // Forward remaining data
                    client_send_queue.send(buf.to_vec());
                    buf.clear();

                    // Keep reading until we have at least 2 bytes
                    loop {
                        // Read packet from socket
                        let mut tmp = Vec::new();
                        ro.read_buf(&mut tmp).await;
                        if tmp.is_empty() {
                            break;
                        }
                        client_send_queue.send(tmp);
                    }

                    // Forward raw packet to server
                    // wi.writable().await;
                    // io::copy(&mut ro, &mut wi).await.map_err(|_| ())?;
                    break;
                }

                // Read packet from stream
                let (packet, raw) = match read_packet(&mut buf, &mut ro).await {
                    Ok(Some(packet)) => packet,
                    Ok(None) => {
                        eprintln!("Closing connection, could not read more");
                        break;
                    }
                    Err(_) => {
                        // Forward raw packet to server
                        client_send_queue.send(buf.to_vec());
                        continue;
                    }
                };

                client_send_queue.send(raw);
            }

            Ok(())
        };

        // Push client sending queue to client
        let send_queue = async {
            wi.writable().await.map_err(|_| ())?;

            while let Some(msg) = client_to_send.recv().await {
                // eprintln!("TO CLIENT: {:?}", &msg);
                wi.write_all(msg.as_ref()).await.map_err(|_| ())?;
            }

            Ok(())
        };

        tokio::try_join!(proxy, send_queue)?;

        io::copy(&mut ro, &mut wi).await.map_err(|_| ())?;

        wi.shutdown().await.map_err(|_| ())
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}
