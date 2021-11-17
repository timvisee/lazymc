use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::mc::ban::{self, BannedIps};
use crate::proto::client::Client;
use crate::proxy;
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

    // Load banned IPs
    server.set_banned_ips(load_banned_ips(&config)).await;

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

    // Spawn server monitor and signal handler services
    tokio::spawn(service::monitor::service(config.clone(), server.clone()));
    tokio::spawn(service::signal::service(config.clone(), server.clone()));

    // Initiate server start
    if config.server.wake_on_start {
        Server::start(config.clone(), server.clone(), None).await;
    }

    // Route all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        route(inbound, config.clone(), server.clone());
    }

    Ok(())
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

    // Check ban state, just drop connection if enabled
    let banned = server.is_banned_ip_blocking(&peer.ip());
    if config.server.drop_banned_ips {
        warn!(target: "lazymc", "Connection from banned IP {}, dropping", peer.ip());
        return;
    }

    // Route connection through proper channel
    let should_proxy =
        !banned && server.state() == server::State::Started && !config.lockout.enabled;
    if should_proxy {
        route_proxy(inbound, config)
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
fn route_proxy(inbound: TcpStream, config: Arc<Config>) {
    // When server is online, proxy all
    let service = proxy::proxy(inbound, config.server.address).map(|r| {
        if let Err(err) = r {
            warn!(target: "lazymc", "Failed to proxy: {}", err);
        }
    });

    tokio::spawn(service);
}

/// Route inbound TCP stream to proxy with queued data, spawning a new task.
#[inline]
pub fn route_proxy_queue(inbound: TcpStream, config: Arc<Config>, queue: BytesMut) {
    route_proxy_address_queue(inbound, config.server.address, queue);
}

/// Route inbound TCP stream to proxy with given address and queued data, spawning a new task.
#[inline]
pub fn route_proxy_address_queue(inbound: TcpStream, addr: SocketAddr, queue: BytesMut) {
    // When server is online, proxy all
    let service = async move {
        proxy::proxy_with_queue(inbound, addr, &queue)
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "lazymc", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}

/// Load banned IPs if IP banning is enabled.
///
/// If disabled or on error, an empty list is returned.
fn load_banned_ips(config: &Config) -> BannedIps {
    // Blocking banned IPs must be enabled
    if !config.server.block_banned_ips && !config.server.drop_banned_ips {
        return BannedIps::default();
    }

    // Ensure server directory is set, it must exist
    let dir = match &config.server.directory {
        Some(dir) => dir,
        None => {
            warn!(target: "lazymc", "Not blocking banned IPs, server directory not configured, unable to find {} file", ban::FILE);
            return BannedIps::default();
        }
    };

    // Determine file path, ensure it exists
    let path = dir.join(crate::mc::ban::FILE);
    if !path.is_file() {
        warn!(target: "lazymc", "Not blocking banned IPs, {} file does not exist", ban::FILE);
        return BannedIps::default();
    }

    // Load banned IPs
    let banned_ips = match ban::load(&path) {
        Ok(ips) => ips,
        Err(err) => {
            // TODO: quit here, require user to disable feature as security feature?
            error!(target: "lazymc", "Failed to load banned IPs from {}: {}", ban::FILE, err);
            return BannedIps::default();
        }
    };

    banned_ips
}
