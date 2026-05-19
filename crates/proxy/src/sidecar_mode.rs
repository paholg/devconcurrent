//! Sidecar mode of the proxy binary. The same image runs in this mode when
//! invoked with the `sidecar` subcommand; the proxy creates these sidecars
//! inside the netns of each compose service that needs port remapping (or
//! TLS termination).
//!
//! The plan + optional cert/key are written into `/etc/sidecar/` by the
//! proxy before the container starts. We read them once on boot and spawn
//! one tokio task per listener.

use std::convert::Infallible;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use eyre::{Context, Result};
use http_body_util::{BodyExt, Empty, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::header::{HOST, HeaderName, HeaderValue};
use hyper::http::uri::PathAndQuery;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use shared::{
    SIDECAR_CERT_FILE, SIDECAR_KEY_FILE, SIDECAR_PLAN_DIR, SIDECAR_PLAN_FILE, SidecarPlan,
};
use tokio::io::{AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::info;

/// Entry point for sidecar mode.
pub async fn run() -> Result<()> {
    let dir = PathBuf::from(SIDECAR_PLAN_DIR);
    let plan_path = dir.join(SIDECAR_PLAN_FILE);
    let plan_bytes =
        std::fs::read(&plan_path).wrap_err_with(|| format!("read {}", plan_path.display()))?;
    let plan: SidecarPlan = serde_json::from_slice(&plan_bytes).wrap_err("parse sidecar plan")?;
    info!(
        hostname = %plan.hostname,
        ports = plan.ports.len(),
        "sidecar starting"
    );

    let tls_acceptor = load_tls(&dir, &plan.hostname);

    let mut tasks = Vec::new();
    for port in plan.ports {
        if port.tls {
            let Some(acceptor) = tls_acceptor.clone() else {
                tracing::warn!(
                    host = port.host,
                    container = port.container,
                    "TLS port skipped: no usable cert"
                );
                continue;
            };
            tasks.push(tokio::spawn(serve_tls(port.host, port.container, acceptor)));
        } else {
            tasks.push(tokio::spawn(serve_plain(port.host, port.container)));
        }
    }

    if tasks.is_empty() {
        eyre::bail!("no listeners configured");
    }

    // If any listener task exits, the sidecar exits — the proxy/docker
    // lifecycle will recreate it.
    let (result, _idx, _rest) = futures_util::future::select_all(tasks).await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(eyre::eyre!("listener task panicked: {e}")),
    }
}

fn load_tls(dir: &Path, hostname: &str) -> Option<TlsAcceptor> {
    let cert_path = dir.join(SIDECAR_CERT_FILE);
    let key_path = dir.join(SIDECAR_KEY_FILE);
    if !cert_path.exists() || !key_path.exists() {
        return None;
    }
    let certs: Vec<CertificateDer<'static>> = match CertificateDer::pem_file_iter(&cert_path)
        .and_then(|it| it.collect::<Result<_, _>>())
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(hostname, path = %cert_path.display(), "load cert: {e}");
            return None;
        }
    };
    let key: PrivateKeyDer<'static> = match PrivateKeyDer::from_pem_file(&key_path) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(hostname, path = %key_path.display(), "load key: {e}");
            return None;
        }
    };
    let config = match ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(hostname, "build rustls config: {e}");
            return None;
        }
    };
    Some(TlsAcceptor::from(Arc::new(config)))
}

async fn serve_plain(host: u16, container: u16) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", host))
        .await
        .wrap_err_with(|| format!("bind 0.0.0.0:{host}"))?;
    info!(host, container, "plain listener ready");
    loop {
        let (mut inbound, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(host, "accept: {e}");
                continue;
            }
        };
        tokio::spawn(async move {
            let mut outbound = match TcpStream::connect(("127.0.0.1", container)).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(container, peer = %peer, "upstream connect: {e}");
                    let _ = inbound.shutdown().await;
                    return;
                }
            };
            let _ = copy_bidirectional(&mut inbound, &mut outbound).await;
        });
    }
}

/// Boxed body type used for both proxied responses (`Incoming`) and locally-
/// generated error responses (`Empty<Bytes>`); the box hides which one we're
/// returning from hyper's serve_connection.
type ProxyBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync + 'static>>;

async fn serve_tls(host: u16, container: u16, acceptor: TlsAcceptor) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", host))
        .await
        .wrap_err_with(|| format!("bind 0.0.0.0:{host}"))?;
    info!(host, container, "tls listener ready");

    let client: Client<HttpConnector, Incoming> =
        Client::builder(TokioExecutor::new()).build_http();
    let upstream = Arc::new(format!("127.0.0.1:{container}"));

    loop {
        let (raw, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(host, "accept: {e}");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let client = client.clone();
        let upstream = upstream.clone();
        tokio::spawn(async move {
            let stream = match acceptor.accept(raw).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(host, peer = %peer, "tls handshake: {e}");
                    return;
                }
            };
            let service = service_fn(move |req| {
                let client = client.clone();
                let upstream = upstream.clone();
                async move { forward_https(req, client, upstream, peer.ip()).await }
            });
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                tracing::debug!(host, peer = %peer, "http1 conn: {e}");
            }
        });
    }
}

/// Forward one decrypted request to `127.0.0.1:<container>`, annotating with
/// X-Forwarded-Proto/Host/For so the app can reconstruct the user-facing URL
/// as `https://…`. Hop-by-hop headers are stripped per RFC 7230 §6.1.
async fn forward_https(
    mut req: Request<Incoming>,
    client: Client<HttpConnector, Incoming>,
    upstream: Arc<String>,
    peer_ip: IpAddr,
) -> std::result::Result<Response<ProxyBody>, Infallible> {
    let path_and_query = req
        .uri()
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));
    let new_uri: Uri = match format!("http://{upstream}{}", path_and_query.as_str()).parse() {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("bad upstream uri: {e}");
            return Ok(error_response(StatusCode::BAD_GATEWAY));
        }
    };
    *req.uri_mut() = new_uri;

    let forwarded_host = req.headers().get(HOST).cloned();
    strip_hop_by_hop(req.headers_mut());

    let xfp = HeaderName::from_static("x-forwarded-proto");
    let xfh = HeaderName::from_static("x-forwarded-host");
    let xff = HeaderName::from_static("x-forwarded-for");
    req.headers_mut().insert(xfp, HeaderValue::from_static("https"));
    if let Some(h) = forwarded_host {
        req.headers_mut().insert(xfh, h);
    }
    if let Ok(val) = HeaderValue::from_str(&peer_ip.to_string()) {
        req.headers_mut().append(xff, val);
    }

    match client.request(req).await {
        Ok(resp) => Ok(resp.map(|b| {
            BoxBody::new(b.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>))
        })),
        Err(e) => {
            tracing::debug!(upstream = %upstream, "forward: {e}");
            Ok(error_response(StatusCode::BAD_GATEWAY))
        }
    }
}

fn error_response(status: StatusCode) -> Response<ProxyBody> {
    let body = Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed();
    Response::builder()
        .status(status)
        .body(body)
        .expect("static response is valid")
}

fn strip_hop_by_hop(headers: &mut hyper::HeaderMap) {
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}
