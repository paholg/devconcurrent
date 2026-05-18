use std::net::IpAddr;
use std::path::Path;

use docker::Docker;
use eyre::{Result, WrapErr};
use shared::{ENV_BIND_ADDRESS, ENV_DNS_PORT, PROJECT_LABEL, PROXY_CONFIG_DIR, ProjectProxyConfig};
use tracing::info;

mod dns;
mod events;
mod http_proxy;
mod registry;
mod routing;
mod sidecar;
mod tcp_listener;

use registry::Registry;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let bind_address: IpAddr = std::env::var(ENV_BIND_ADDRESS)
        .wrap_err_with(|| format!("{ENV_BIND_ADDRESS} not set"))?
        .parse()
        .wrap_err_with(|| format!("invalid {ENV_BIND_ADDRESS}"))?;
    let dns_port: u16 = std::env::var(ENV_DNS_PORT)
        .wrap_err_with(|| format!("{ENV_DNS_PORT} not set"))?
        .parse()
        .wrap_err_with(|| format!("invalid {ENV_DNS_PORT}"))?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        %bind_address,
        dns_port,
        "devconcurrent-proxy starting"
    );

    let docker = Docker::connect()
        .await
        .wrap_err("connect to docker daemon")?;

    let registry = Registry::new();
    let configs = read_project_configs(Path::new(PROXY_CONFIG_DIR))?;
    info!(count = configs.len(), "loaded project configs");
    registry.load_configs(configs).await;

    // Adopt any already-running workspace containers as if they'd just started.
    bootstrap_running_workspaces(&docker, &registry).await?;
    // Drop sidecars whose targets no longer exist.
    if let Err(e) = sidecar::sweep_orphans(&docker).await {
        tracing::warn!("orphan sweep failed: {e:?}");
    }

    let dns_bind = dns::dns_bind(bind_address, dns_port);
    let http_bind = http_proxy::http_bind(bind_address);

    let dns_task = tokio::spawn({
        let registry = registry.clone();
        async move {
            if let Err(e) = dns::serve(dns_bind, registry, bind_address).await {
                tracing::error!("dns server stopped: {e:?}");
            }
        }
    });
    let http_task = tokio::spawn({
        let registry = registry.clone();
        async move {
            if let Err(e) = http_proxy::serve(http_bind, registry).await {
                tracing::error!("http server stopped: {e:?}");
            }
        }
    });
    let tcp_tasks = spawn_tcp_listeners(bind_address, &registry).await;
    let events_task = tokio::spawn({
        let docker = docker.clone();
        let registry = registry.clone();
        async move { events::run(docker, registry).await }
    });

    tokio::select! {
        _ = dns_task => tracing::error!("dns task exited"),
        _ = http_task => tracing::error!("http task exited"),
        _ = events_task => tracing::error!("events task exited"),
    }
    drop(tcp_tasks);
    Ok(())
}

fn read_project_configs(dir: &Path) -> Result<Vec<ProjectProxyConfig>> {
    let mut out = Vec::new();
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).wrap_err_with(|| format!("read {}", dir.display())),
    };
    for entry in read_dir {
        let entry = entry.wrap_err("read dir entry")?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path).wrap_err_with(|| format!("read {}", path.display()))?;
        match serde_json::from_slice::<ProjectProxyConfig>(&bytes) {
            Ok(cfg) => out.push(cfg),
            Err(e) => tracing::warn!(path = %path.display(), "invalid project config: {e}"),
        }
    }
    Ok(out)
}

async fn bootstrap_running_workspaces(docker: &Docker, registry: &Registry) -> Result<()> {
    let containers = docker
        .list_containers()
        .with_label_key(PROJECT_LABEL)
        .call()
        .await
        .wrap_err("list workspace containers")?;
    for c in containers {
        let project = match c.labels.get(PROJECT_LABEL) {
            Some(p) => p.clone(),
            None => continue,
        };
        let service = c
            .labels
            .get(shared::COMPOSE_SERVICE_LABEL)
            .cloned()
            .unwrap_or_default();

        let Some(cfg) = registry.config_for(&project).await else {
            continue;
        };
        if service != cfg.devcontainer_service {
            continue;
        }
        let Some(workspace) = events::derive_workspace(&c.labels) else {
            tracing::debug!(
                container = %c.id,
                project,
                "container has project label but no workspace identifier; skipping"
            );
            continue;
        };

        let sidecar_id = match sidecar::create_sidecar(docker, &cfg, &workspace, &c.id).await {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::error!(project, workspace, "bootstrap create sidecar: {e:?}");
                None
            }
        };
        registry
            .track_workspace(registry::RunningWorkspace {
                project,
                workspace,
                target_cid: c.id.clone(),
                sidecar_id,
            })
            .await;
    }
    Ok(())
}

async fn spawn_tcp_listeners(
    bind_address: IpAddr,
    registry: &Registry,
) -> Vec<tokio::task::JoinHandle<()>> {
    let ports = registry.configured_tcp_ports().await;
    let mut handles = Vec::with_capacity(ports.len());
    for port in ports {
        let registry = registry.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = tcp_listener::serve_port(bind_address, port, registry).await {
                tracing::error!(port, "tcp listener stopped: {e:?}");
            }
        });
        handles.push(handle);
    }
    handles
}
