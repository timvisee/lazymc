use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::proto::Client;
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

    // Listen for new connections
    // TODO: do not drop error here
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

    // Spawn server monitor and signal handler services
    tokio::spawn(service::monitor::service(config.clone(), server.clone()));
    tokio::spawn(service::signal::service(config.clone(), server.clone()));

    // Initiate server start
    if config.server.wake_on_start {
        Server::start(config.clone(), server.clone(), None);
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
    if server.state() == server::State::Started {
        route_proxy(inbound, config)
    } else {
        route_status(inbound, config, server)
    }
}

/// Route inbound TCP stream to status server, spawning a new task.
#[inline]
fn route_status(inbound: TcpStream, config: Arc<Config>, server: Arc<Server>) {
    // When server is not online, spawn a status server
    let client = Client::default();
    let service = status::serve(client, inbound, config.clone(), server.clone()).map(|r| {
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
pub fn route_proxy_queue<'a>(inbound: TcpStream, config: Arc<Config>, queue: BytesMut) {
    // When server is online, proxy all
    let service = async move {
        proxy::proxy_with_queue(inbound, config.server.address, &queue)
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "lazymc", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}
