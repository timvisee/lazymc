use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::proto::client::Client;
use crate::proxy::{self, ProxyHeader};
use crate::router::Router;
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
            route(inbound, router.clone());
        }
    }

    Ok(())
}

/// Route inbound TCP stream to correct service, spawning a new task.
#[inline]
fn route(inbound: TcpStream, router: Arc<Router>) {
    // Get user peer address
    let peer = match inbound.peer_addr() {
        Ok(peer) => peer,
        Err(err) => {
            warn!(target: "lazymc", "Connection from unknown peer address, disconnecting: {}", err);
            return;
        }
    };

    let client = Client::new(peer);
    let service = status::serve(client, inbound, router).map(|r| {
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
