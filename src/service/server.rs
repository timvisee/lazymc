use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::proto::client::Client;
use crate::proxy::{self, ProxyHeader};
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
pub async fn service(config: Arc<Config>) -> Result<(), ()> {
    // Load server state
    let server = Arc::new(Server::default());

    // Listen for new connections
    let listener = TcpListener::bind(config.public.address)
        .await
        .map_err(|err| {
            quit_error(
                anyhow!(err).context("Failed to start proxy server"),
                ErrorHints::default(),
            );
        })?;

    info!(
        target: "lazymc",
        "Proxying public {} to server {}",
        config.public.address, config.server.address,
    );

    if config.lockout.enabled {
        warn!(
            target: "lazymc",
            "Lockout mode is enabled, nobody will be able to connect through the proxy",
        );
    }

    // Spawn services: monitor, signal handler
    tokio::spawn(service::monitor::service(config.clone(), server.clone()));
    tokio::spawn(service::signal::service(config.clone(), server.clone()));

    // Initiate server start
    if config.server.wake_on_start {
        Server::start(config.clone(), server.clone(), None).await;
    }

    // Spawn additional services: probe and ban manager
    tokio::spawn(service::probe::service(config.clone(), server.clone()));
    tokio::task::spawn_blocking({
        let (config, server) = (config.clone(), server.clone());
        || service::file_watcher::service(config, server)
    });

    // The PROXY listener, for a trusted proxy in front (public.proxy_address).
    if let Some(address) = &config.public.proxy_address {
        let address: SocketAddr = address.parse().map_err(|err| {
            quit_error(
                anyhow!("public.proxy_address {address:?} isn't an address: {err}"),
                ErrorHints::default(),
            );
        })?;
        let proxied = TcpListener::bind(address).await.map_err(|err| {
            quit_error(
                anyhow!(err).context("Failed to start the PROXY listener"),
                ErrorHints::default(),
            );
        })?;
        info!(target: "lazymc", "Accepting PROXY v2 connections on {}", address);
        let (config, server) = (config.clone(), server.clone());
        tokio::spawn(async move {
            while let Ok((inbound, _)) = proxied.accept().await {
                route_proxied(inbound, config.clone(), server.clone());
            }
        });
    }

    // Route all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        route(inbound, config.clone(), server.clone());
    }

    Ok(())
}

/// A connection on the PROXY listener: its client address comes from the PROXY v2 header it must start with.
/// No valid header in time, and it's dropped.
fn route_proxied(mut inbound: TcpStream, config: Arc<Config>, server: Arc<Server>) {
    tokio::spawn(async move {
        let peer = match proxy::read_proxy_header(&mut inbound).await {
            Ok(Some(client)) => client,
            Ok(None) => match inbound.peer_addr() {
                Ok(peer) => peer,
                Err(_) => return,
            },
            Err(err) => {
                warn!(target: "lazymc", "Dropping connection on the PROXY listener: {}", err);
                return;
            }
        };
        route_peer(inbound, peer, config, server);
    });
}

/// Route inbound TCP stream to correct service, spawning a new task.
#[inline]
fn route(inbound: TcpStream, config: Arc<Config>, server: Arc<Server>) {
    // Get user peer address
    let peer = match inbound.peer_addr() {
        Ok(peer) => peer,
        Err(err) => {
            warn!(target: "lazymc", "Connection from unknown peer address, disconnecting: {}", err);
            return;
        }
    };
    route_peer(inbound, peer, config, server);
}

/// Route a stream whose client is `peer` (on the PROXY listener: the client its PROXY header gave).
#[inline]
fn route_peer(inbound: TcpStream, peer: SocketAddr, config: Arc<Config>, server: Arc<Server>) {
    // Check ban state, just drop connection if enabled
    let banned = server.is_banned_ip_blocking(&peer.ip());
    if banned && config.server.drop_banned_ips {
        info!(target: "lazymc", "Connection from banned IP {}, dropping", peer.ip());
        return;
    }

    // Route connection through proper channel
    let should_proxy =
        !banned && server.state() == server::State::Started && !config.lockout.enabled;
    if should_proxy {
        route_proxy(inbound, config, peer)
    } else {
        route_status(inbound, config, server, peer)
    }
}

/// Route inbound TCP stream to status server, spawning a new task.
#[inline]
fn route_status(inbound: TcpStream, config: Arc<Config>, server: Arc<Server>, peer: SocketAddr) {
    // When server is not online, spawn a status server
    let client = Client::new(peer);
    let service = status::serve(client, inbound, config, server).map(|r| {
        if let Err(err) = r {
            warn!(target: "lazymc", "Failed to serve status: {:?}", err);
        }
    });

    tokio::spawn(service);
}

/// Route inbound TCP stream to proxy, spawning a new task.
#[inline]
fn route_proxy(inbound: TcpStream, config: Arc<Config>, client: SocketAddr) {
    // When server is online, proxy all
    let service = proxy::proxy(
        inbound,
        ProxyHeader::Proxy.not_none(config.server.send_proxy_v2),
        config.server.address,
        client,
    )
    .map(|r| {
        if let Err(err) = r {
            warn!(target: "lazymc", "Failed to proxy: {}", err);
        }
    });

    tokio::spawn(service);
}

/// Route inbound TCP stream to proxy with queued data, spawning a new task.
#[inline]
pub fn route_proxy_queue(
    inbound: TcpStream,
    config: Arc<Config>,
    queue: BytesMut,
    client: SocketAddr,
) {
    route_proxy_address_queue(
        inbound,
        ProxyHeader::Proxy.not_none(config.server.send_proxy_v2),
        config.server.address,
        queue,
        client,
    );
}

/// Route inbound TCP stream to proxy with given address and queued data, spawning a new task.
#[inline]
pub fn route_proxy_address_queue(
    inbound: TcpStream,
    proxy_header: ProxyHeader,
    addr: SocketAddr,
    queue: BytesMut,
    client: SocketAddr,
) {
    // When server is online, proxy all
    let service = async move {
        proxy::proxy_with_queue(inbound, proxy_header, addr, &queue, client)
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "lazymc", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}
