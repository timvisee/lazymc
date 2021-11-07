pub(crate) mod config;
pub(crate) mod monitor;
pub(crate) mod protocol;
pub(crate) mod server;
pub(crate) mod types;

use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use minecraft_protocol::data::chat::{Message, Payload};
use minecraft_protocol::data::server_status::*;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::LoginDisconnect;
use minecraft_protocol::version::v1_14_4::status::StatusResponse;
use tokio::io;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::ReadHalf;
use tokio::net::{TcpListener, TcpStream};

use config::*;
use protocol::{Client, ClientState, RawPacket};
use server::ServerState;

#[tokio::main]
async fn main() -> Result<(), ()> {
    println!(
        "Proxying public {} to internal {}",
        ADDRESS_PUBLIC, ADDRESS_PROXY,
    );

    let server_state = Arc::new(ServerState::default());

    // Listen for new connections
    // TODO: do not drop error here
    let listener = TcpListener::bind(ADDRESS_PUBLIC).await.map_err(|_| ())?;

    // Spawn server monitor
    let addr = ADDRESS_PROXY.parse().expect("invalid server IP");
    tokio::spawn(monitor::monitor_server(addr, server_state.clone()));

    let sub = server_state.clone();
    tokio::spawn(async move {
        loop {
            tokio::signal::ctrl_c().await.unwrap();
            if !sub.kill_server() {
                // TODO: gracefully kill itself instead
                std::process::exit(1)
            }
        }
    });

    // Proxy all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        let client = Client::default();
        // eprintln!("Client connected");

        if !server_state.online() {
            // When server is not online, spawn a status server
            let transfer = status_server(client, inbound, server_state.clone()).map(|r| {
                if let Err(e) = r {
                    println!("Failed to proxy: {:?}", e);
                }

                // eprintln!("Client disconnected");
            });

            tokio::spawn(transfer);
        } else {
            // When server is online, proxy all
            let transfer = proxy(inbound, ADDRESS_PROXY.to_string()).map(|r| {
                if let Err(e) = r {
                    println!("Failed to proxy: {:?}", e);
                }

                // eprintln!("Client disconnected");
            });

            tokio::spawn(transfer);
        }
    }

    Ok(())
}

/// Read raw packet from stream.
pub async fn read_packet<'a>(
    buf: &mut BytesMut,
    stream: &mut ReadHalf<'a>,
) -> Result<Option<(RawPacket, Vec<u8>)>, ()> {
    // Keep reading until we have at least 2 bytes
    while buf.len() < 2 {
        // Read packet from socket
        let mut tmp = Vec::with_capacity(64);
        stream.read_buf(&mut tmp).await.map_err(|_| ())?;
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
        stream.read_buf(&mut tmp).await.map_err(|_| ())?;
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
async fn status_server(
    client: Client,
    mut inbound: TcpStream,
    server: Arc<ServerState>,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // Incoming buffer
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, raw) = match read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => {
                eprintln!("Closing connection, could not read more");
                break;
            }
            Err(_) => {
                eprintln!("Closing connection, error occurred");
                break;
            }
        };

        // Hijack login start
        if client.state() == ClientState::Login
            && packet.id == protocol::LOGIN_PACKET_ID_LOGIN_START
        {
            let packet = LoginDisconnect {
                reason: Message::new(Payload::text(LABEL_SERVER_STARTING_MESSAGE)),
            };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(0, data).encode()?;

            writer.write_all(&response).await.map_err(|_| ())?;

            // Start server if not starting yet
            if !server.starting() {
                server.set_starting(true);
                tokio::spawn(server::start(server).map(|_| ()));
            }

            break;
        }

        // Hijack handshake
        if client.state() == ClientState::Handshake
            && packet.id == protocol::STATUS_PACKET_ID_STATUS
        {
            match Handshake::decode(&mut packet.data.as_slice()) {
                Ok(handshake) => {
                    // TODO: do not panic here
                    client.set_state(
                        ClientState::from_id(handshake.next_state)
                            .expect("unknown next client state"),
                    );
                }
                Err(_) => break,
            }
        }

        // Hijack server status packet
        if client.state() == ClientState::Status && packet.id == protocol::STATUS_PACKET_ID_STATUS {
            // Build status response
            // TODO: grab latest protocol version from online server!
            let description = if server.starting() {
                LABEL_SERVER_STARTING
            } else {
                LABEL_SERVER_SLEEPING
            };
            let server_status = ServerStatus {
                version: ServerVersion {
                    name: String::from("1.16.5"),
                    protocol: 754,
                },
                description: Message::new(Payload::text(description)),
                players: OnlinePlayers {
                    online: 0,
                    max: 0,
                    sample: vec![],
                },
            };
            let packet = StatusResponse { server_status };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(0, data).encode()?;
            writer.write_all(&response).await.map_err(|_| ())?;
            continue;
        }

        // Hijack ping packet
        if client.state() == ClientState::Status && packet.id == protocol::STATUS_PACKET_ID_PING {
            writer.write_all(&raw).await.map_err(|_| ())?;
            continue;
        }

        // // Show unhandled packet warning
        // eprintln!("Received unhandled packet:");
        // eprintln!("- State: {:?}", client.state());
        // eprintln!("- Packet ID: {}", packet.id);
    }

    // Gracefully close connection
    writer.shutdown().await.map_err(|_| ())?;

    Ok(())
}

/// Proxy the inbound stream to a target address.
// TODO: do not drop error here, return Box<dyn Error>
async fn proxy(mut inbound: TcpStream, addr_target: String) -> Result<(), ()> {
    let mut outbound = TcpStream::connect(addr_target).await.map_err(|_| ())?;

    // TODO: on connect fail, ping server and redirect to status_server if offline

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    let client_to_server = async {
        io::copy(&mut ri, &mut wo).await.map_err(|_| ())?;
        wo.shutdown().await.map_err(|_| ())
    };

    let server_to_client = async {
        io::copy(&mut ro, &mut wi).await.map_err(|_| ())?;
        wi.shutdown().await.map_err(|_| ())
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}
