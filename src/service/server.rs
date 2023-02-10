use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::proto::client::Client;
use crate::proxy::{self, ProxyHeader};
use crate::server::Server;
use crate::service;
use crate::status;
use crate::util::error::{quit_error, ErrorHints};

/// Start lazymc.
///
/// Main entrypoint to start all server/status/proxy logic.
///
/// Spawns a tokio runtime to complete all work on.
#[tokio::main(flavor = "multi_thread")]
pub async fn service(bind_addr: SocketAddr, configs: Vec<Arc<Config>>) -> Result<(), ()> {
    // Listen for new connections
    let listener = TcpListener::bind(bind_addr).await.map_err(|err| {
        quit_error(
            anyhow!(err).context("Failed to start proxy server"),
            ErrorHints::default(),
        );
    })?;

    let mut servers = HashMap::new();

    for config in configs.iter() {
        // Load server state
        let server = Arc::new(Server::default());
        servers.insert(
            config.public.address.clone(),
            (config.clone(), server.clone()),
        );

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
    }

    let servers_arc = Arc::new(servers);

    // Route all incomming connections
    while let Ok((inbound, _)) = listener.accept().await {
        route(inbound, servers_arc.clone());
    }

    Ok(())
}

/// Route inbound TCP stream to correct service, spawning a new task.
#[inline]
fn route(inbound: TcpStream, servers: Arc<HashMap<String, (Arc<Config>, Arc<Server>)>>) {
    // Get user peer address
    let peer = match inbound.peer_addr() {
        Ok(peer) => peer,
        Err(err) => {
            warn!(target: "lazymc", "Connection from unknown peer address, disconnecting: {}", err);
            return;
        }
    };

    let client = Client::new(peer);
    let service = status::serve(client, inbound, servers).map(|r| {
        if let Err(err) = r {
            warn!(target: "lazymc", "Failed to serve status: {:?}", err);
        }
    });

    tokio::spawn(service);
}

/// Route inbound TCP stream to proxy with queued data, spawning a new task.
#[inline]
pub fn route_proxy_queue(inbound: TcpStream, config: Arc<Config>, queue: BytesMut) {
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
