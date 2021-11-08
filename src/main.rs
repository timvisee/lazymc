#[macro_use]
extern crate log;

pub(crate) mod config;
pub(crate) mod monitor;
pub(crate) mod proto;
pub(crate) mod server;
pub(crate) mod types;

use std::error::Error;
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
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use config::*;
use proto::{Client, ClientState, RawPacket, PROTO_DEFAULT_PROTOCOL, PROTO_DEFAULT_VERSION};
use server::ServerState;

#[tokio::main]
async fn main() -> Result<(), ()> {
    // Initialize logging
    let _ = dotenv::dotenv();
    pretty_env_logger::init();

    let server_state = Arc::new(ServerState::default());

    // Listen for new connections
    // TODO: do not drop error here
    let listener = TcpListener::bind(ADDRESS_PUBLIC).await.map_err(|err| {
        error!("Failed to start: {}", err);
        ()
    })?;

    info!(
        "Proxying egress {} to ingress {}",
        ADDRESS_PUBLIC, ADDRESS_PROXY,
    );

    // Spawn server monitor and signal handler
    tokio::spawn(server_monitor(server_state.clone()));
    tokio::spawn(signal_handler(server_state.clone()));

    // Proxy all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        let client = Client::default();

        if !server_state.online() {
            // When server is not online, spawn a status server
            let transfer = serve_status(client, inbound, server_state.clone()).map(|r| {
                if let Err(err) = r {
                    error!("Failed to serve status: {:?}", err);
                }
            });

            tokio::spawn(transfer);
        } else {
            // When server is online, proxy all
            let transfer = proxy(inbound, ADDRESS_PROXY.to_string()).map(|r| {
                if let Err(err) = r {
                    error!("Failed to proxy: {}", err);
                }
            });

            tokio::spawn(transfer);
        }
    }

    Ok(())
}

/// Signal handler task.
pub async fn signal_handler(server_state: Arc<ServerState>) {
    loop {
        tokio::signal::ctrl_c().await.unwrap();
        if !server_state.kill_server() {
            // TODO: gracefully kill itself instead
            std::process::exit(1)
        }
    }
}

/// Server monitor task.
pub async fn server_monitor(state: Arc<ServerState>) {
    let addr = ADDRESS_PROXY.parse().expect("invalid server IP");
    monitor::monitor_server(addr, state).await
}

/// Proxy the given inbound stream to a target address.
// TODO: do not drop error here, return Box<dyn Error>
async fn serve_status(
    client: Client,
    mut inbound: TcpStream,
    server: Arc<ServerState>,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    // Incoming buffer
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, raw) = match proto::read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!("Closing connection, error occurred");
                break;
            }
        };

        // Hijack login start
        if client.state() == ClientState::Login && packet.id == proto::LOGIN_PACKET_ID_LOGIN_START {
            let packet = LoginDisconnect {
                reason: Message::new(Payload::text(LABEL_SERVER_STARTING_MESSAGE)),
            };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(0, data).encode()?;

            writer.write_all(&response).await.map_err(|_| ())?;

            // Start server if not starting yet
            // TODO: move this into server state?
            if !server.starting() {
                server.set_starting(true);
                server.update_last_active_time();
                tokio::spawn(server::start(server).map(|_| ()));
            }

            break;
        }

        // Hijack handshake
        if client.state() == ClientState::Handshake && packet.id == proto::STATUS_PACKET_ID_STATUS {
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
        if client.state() == ClientState::Status && packet.id == proto::STATUS_PACKET_ID_STATUS {
            // Select version and player max from last known server status
            let (version, max) = match server.clone_status() {
                Some(status) => (status.version, status.players.max),
                None => (
                    ServerVersion {
                        name: String::from(PROTO_DEFAULT_VERSION),
                        protocol: PROTO_DEFAULT_PROTOCOL,
                    },
                    0,
                ),
            };

            // Select description
            let description = if server.starting() {
                LABEL_SERVER_STARTING
            } else {
                LABEL_SERVER_SLEEPING
            };

            // Build status resposne
            let server_status = ServerStatus {
                version,
                description: Message::new(Payload::text(description)),
                players: OnlinePlayers {
                    online: 0,
                    max,
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
        if client.state() == ClientState::Status && packet.id == proto::STATUS_PACKET_ID_PING {
            writer.write_all(&raw).await.map_err(|_| ())?;
            continue;
        }

        // Show unhandled packet warning
        debug!("Received unhandled packet:");
        debug!("- State: {:?}", client.state());
        debug!("- Packet ID: {}", packet.id);
    }

    // Gracefully close connection
    match writer.shutdown().await {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotConnected => {}
        Err(_) => return Err(()),
    }

    Ok(())
}

/// Proxy the inbound stream to a target address.
async fn proxy(mut inbound: TcpStream, addr_target: String) -> Result<(), Box<dyn Error>> {
    // Set up connection to server
    // TODO: on connect fail, ping server and redirect to serve_status if offline
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
