use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::{packet, packets};
use crate::proxy::{self, ProxyHeader};
use crate::router::Router;
use crate::server::{self, Server};
use crate::service;
use crate::status;
use crate::util::error::{quit_error, ErrorHints};

/// Start lazymc.
///
/// Main entrypoint to start all server/status/proxy logic.
///
/// Spawns a tokio runtime to complete all work on.
#[tokio::main(flavor = "multi_thread")]
pub async fn service(configs: Vec<Config>) -> Result<(), ()> {
    let mut routers: HashMap<SocketAddr, Router> = HashMap::new();

    for config in configs.into_iter() {
        // Load server state
        let server = Arc::new(Server::new(config));
        routers
            .entry(server.config.public.address)
            .or_default()
            .data
            .insert(server.config.server.name.clone(), server.clone());

        if server.config.lockout.enabled {
            warn!(
                target: "lazymc",
                "Lockout mode is enabled, nobody will be able to connect through the proxy",
            );
        }

        // Spawn services: monitor, signal handler
        tokio::spawn(service::monitor::service(server.clone()));
        tokio::spawn(service::signal::service(server.clone()));

        // Initiate server start
        if server.config.server.wake_on_start {
            Server::start(server.clone(), None).await;
        }

        // Spawn additional services: probe and ban manager
        tokio::spawn(service::probe::service(server.clone()));
        tokio::task::spawn_blocking({
            let server = server.clone();
            || service::file_watcher::service(server)
        });
    }

    info!(target: "lazymc", "Routing\n{}", routers.iter().flat_map(|(public_address, router)| {
        router.data.iter().map(move |(server_name, server)| {
            format!("{} -> {} -> {}", server_name.clone().unwrap_or("*".to_string()), public_address, server.config.server.address.clone())
        })
    }).collect::<Vec<String>>().join("\n"));

    for (public_address, router) in routers {
        let listener = TcpListener::bind(public_address).await.map_err(|err| {
            quit_error(
                anyhow!(err).context("Failed to start proxy server"),
                ErrorHints::default(),
            );
        })?;

        let router = Arc::new(router);

        // Route all incomming connections
        while let Ok((inbound, _)) = listener.accept().await {
            route(inbound, router.clone()).await;
        }
    }

    Ok(())
}

/// Route inbound TCP stream to correct service, spawning a new task.
#[inline]
async fn route(mut inbound: TcpStream, router: Arc<Router>) {
    // Get user peer address
    let peer = match inbound.peer_addr() {
        Ok(peer) => peer,
        Err(err) => {
            warn!(target: "lazymc", "Connection from unknown peer address, disconnecting: {}", err);
            return;
        }
    };

    let client = Client::new(peer);

    let (mut reader, _) = inbound.split();

    // Incoming buffer and packet holding queue
    let mut buf = BytesMut::new();

    // Remember inbound packets, track client info
    let mut inbound_history = BytesMut::new();
    let mut client_info = ClientInfo::empty();

    // Read packet from stream
    let (packet, raw) = match packet::read_packet(&client, &mut buf, &mut reader).await {
        Ok(Some(packet)) => packet,
        Ok(None) => return,
        Err(_) => {
            error!(target: "lazymc", "Closing connection, error occurred");
            return;
        }
    };

    // Hijack handshake
    if client.state() == ClientState::Handshake && packet.id == packets::handshake::SERVER_HANDSHAKE
    {
        // Parse handshake
        let handshake = match Handshake::decode(&mut packet.data.as_slice()) {
            Ok(handshake) => handshake,
            Err(_) => {
                debug!(target: "lazymc", "Got malformed handshake from client, disconnecting");
                return;
            }
        };

        let server = match router.get(handshake.server_addr.clone()) {
            Some(server) => server,
            None => {
                error!(target: "lazymc", "Client tried to join a non existing server ({})", handshake.server_addr);
                return;
            }
        };

        // Check ban state, just drop connection if enabled
        let banned = server.is_banned_ip_blocking(&peer.ip());
        if banned && server.config.server.drop_banned_ips {
            info!(target: "lazymc", "Connection from banned IP {}, dropping", peer.ip());
            return;
        }

        // Parse new state
        let new_state = match ClientState::from_id(handshake.next_state) {
            Some(state) => state,
            None => {
                error!(target: "lazymc", "Client tried to switch into unknown protcol state ({}), disconnecting", handshake.next_state);
                return;
            }
        };

        // Update client info and client state
        client_info
            .protocol
            .replace(handshake.protocol_version as u32);
        client_info.handshake.replace(handshake);
        client.set_state(new_state);
        inbound_history.extend(raw);

        // Route connection through proper channel
        let should_proxy =
            !banned && server.state() == server::State::Started && !server.config.lockout.enabled;
        if should_proxy {
            route_proxy_queue(inbound, &server.config, inbound_history)
        } else {
            route_status(client, inbound, server, inbound_history, client_info)
        }
    }
}

/// Route inbound TCP stream to status server, spawning a new task.
#[inline]
fn route_status(
    client: Client,
    inbound: TcpStream,
    server: Arc<Server>,
    inbound_history: BytesMut,
    client_info: ClientInfo,
) {
    // When server is not online, spawn a status server
    let service = status::serve(client, inbound, server, inbound_history, client_info).map(|r| {
        if let Err(err) = r {
            warn!(target: "lazymc", "Failed to serve status: {:?}", err);
        }
    });

    tokio::spawn(service);
}

/// Route inbound TCP stream to proxy with queued data, spawning a new task.
#[inline]
pub fn route_proxy_queue(inbound: TcpStream, config: &Config, queue: BytesMut) {
    route_proxy_address_queue(
        inbound,
        ProxyHeader::Proxy.not_none(config.server.send_proxy_v2),
        config.server.address,
        queue,
    );
}

/// Route inbound TCP stream to proxy with given address and queued data, spawning a new task.
#[inline]
pub fn route_proxy_address_queue(
    inbound: TcpStream,
    proxy_header: ProxyHeader,
    addr: SocketAddr,
    queue: BytesMut,
) {
    // When server is online, proxy all
    let service = async move {
        proxy::proxy_with_queue(inbound, proxy_header, addr, &queue)
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "lazymc", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}
