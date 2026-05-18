//! HTTP reverse proxy bound to the alias's port 80.
//!
//! Resolves the target sidecar's unix socket via the [`Registry`] using the
//! lowercased, port-stripped Host header, then forwards the request through a
//! per-request `hyper` client.

use std::net::{IpAddr, SocketAddr};

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use eyre::{Result, WrapErr};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector, Uri as UnixUri};
use tokio::net::TcpListener;

use crate::registry::Registry;

const BODY_LIMIT: usize = 64 * 1024 * 1024; // 64 MiB cap; bigger uploads fail.

pub async fn serve(bind: SocketAddr, registry: Registry) -> Result<()> {
    let client: Client<UnixConnector, Body> = Client::unix();
    let app = axum::Router::new()
        .fallback(any(handle))
        .with_state(AppState { registry, client });

    let listener = TcpListener::bind(bind)
        .await
        .wrap_err_with(|| format!("bind http listener on {bind}"))?;
    tracing::info!("http proxy listening on {bind}");
    axum::serve(listener, app).await.wrap_err("http server")?;
    Ok(())
}

#[derive(Clone)]
struct AppState {
    registry: Registry,
    client: Client<UnixConnector, Body>,
}

async fn handle(State(state): State<AppState>, req: Request<Body>) -> Response<Body> {
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let Some(route) = state.registry.http_route(&host).await else {
        return (
            StatusCode::NOT_FOUND,
            format!("no route for host: {host:?}\n"),
        )
            .into_response();
    };

    let (parts, body) = req.into_parts();
    let body_bytes = match to_bytes(body, BODY_LIMIT).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("failed to buffer request body: {e}");
            return (StatusCode::BAD_GATEWAY, "bad request body\n").into_response();
        }
    };

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let target_uri: Uri = UnixUri::new(&route.socket_path, path_and_query).into();

    let mut out = Request::builder()
        .method(parts.method.clone())
        .uri(target_uri);
    if let Some(headers) = out.headers_mut() {
        copy_request_headers(&parts.headers, headers, &parts.method);
    }
    let req = match out.body(Body::from(body_bytes)) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("build upstream request: {e}");
            return (StatusCode::BAD_GATEWAY, "upstream build error\n").into_response();
        }
    };

    match state.client.request(req).await {
        Ok(resp) => resp.into_response(),
        Err(e) => {
            tracing::warn!(socket = %route.socket_path, "upstream request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                "upstream unreachable\n".to_string(),
            )
                .into_response()
        }
    }
}

fn copy_request_headers(src: &HeaderMap, dst: &mut HeaderMap, _method: &Method) {
    for (name, value) in src.iter() {
        // Skip hop-by-hop headers as required by RFC 7230.
        let n = name.as_str();
        if matches!(
            n.to_ascii_lowercase().as_str(),
            "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailers"
                | "transfer-encoding"
                | "upgrade"
        ) {
            continue;
        }
        dst.append(name, value.clone());
    }
}

/// Helper for the entry point: build a socket address from `(bind_address, 80)`.
pub fn http_bind(bind_address: IpAddr) -> SocketAddr {
    SocketAddr::new(bind_address, 80)
}
