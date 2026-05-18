//! Plain-TCP forwarders for non-port-80 host ports.
//!
//! Each listener binds `(bind_address, host_port)` and, on accept, looks up the
//! current sidecar socket in the [`Registry`] and bridges bytes both ways.

use std::net::{IpAddr, SocketAddr};

use eyre::{Result, WrapErr};
use tokio::io::{AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream, UnixStream};

use crate::registry::Registry;

pub async fn serve_port(bind_address: IpAddr, host_port: u16, registry: Registry) -> Result<()> {
    let addr = SocketAddr::new(bind_address, host_port);
    let listener = TcpListener::bind(addr)
        .await
        .wrap_err_with(|| format!("bind tcp listener on {addr}"))?;
    tracing::info!("tcp listener on {addr}");
    loop {
        let (client, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(host_port, "accept: {e}");
                continue;
            }
        };
        let registry = registry.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(client, host_port, registry).await {
                tracing::debug!(host_port, peer = %peer, "tcp forward ended: {e}");
            }
        });
    }
}

async fn handle_conn(mut client: TcpStream, host_port: u16, registry: Registry) -> Result<()> {
    let Some(route) = registry.tcp_route(host_port).await else {
        let _ = client.shutdown().await;
        eyre::bail!("no route for host port {host_port}");
    };

    let mut upstream = UnixStream::connect(&route.socket_path)
        .await
        .wrap_err_with(|| format!("connect upstream {}", route.socket_path))?;

    copy_bidirectional(&mut client, &mut upstream)
        .await
        .wrap_err("bidirectional copy")?;
    Ok(())
}
