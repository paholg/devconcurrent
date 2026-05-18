//! Shared in-memory state: pushed project configs + currently-tracked workspaces.
//!
//! The proxy reads `/etc/projects/*.json` from a docker volume on startup, and
//! mutates the workspace map in response to docker container start/die events.
//! Derived routing tables are rebuilt each time the registry changes.

use std::collections::HashMap;
use std::sync::Arc;

use shared::ProjectProxyConfig;
use tokio::sync::RwLock;

/// A single (hostname, host_port) route to a sidecar's unix socket.
#[derive(Debug, Clone)]
pub struct HostRoute {
    /// Sidecar unix socket path, e.g. `/socks/<key>.sock`.
    pub socket_path: String,
}

/// Routes for HTTP traffic on the alias's port 80, keyed by Host header
/// (lowercased, port-stripped).
pub type HttpRoutes = HashMap<String, HostRoute>;
/// Routes for plain TCP traffic on non-HTTP host ports, keyed by host port.
pub type TcpRoutes = HashMap<u16, HostRoute>;

/// One running workspace tracked from docker start events.
#[derive(Debug, Clone)]
pub struct RunningWorkspace {
    pub project: String,
    pub workspace: String,
    pub target_cid: String,
    pub sidecar_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct RegistryInner {
    pub configs: HashMap<String, ProjectProxyConfig>,
    pub workspaces: HashMap<String, RunningWorkspace>,
    pub http_routes: HttpRoutes,
    pub tcp_routes: TcpRoutes,
}

#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<RwLock<RegistryInner>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn load_configs(&self, configs: Vec<ProjectProxyConfig>) {
        let mut inner = self.inner.write().await;
        inner.configs.clear();
        for cfg in configs {
            inner.configs.insert(cfg.project.clone(), cfg);
        }
        rebuild_routes(&mut inner);
    }

    pub async fn config_for(&self, project: &str) -> Option<ProjectProxyConfig> {
        self.inner.read().await.configs.get(project).cloned()
    }

    pub async fn track_workspace(&self, ws: RunningWorkspace) {
        let mut inner = self.inner.write().await;
        inner.workspaces.insert(ws.target_cid.clone(), ws);
        rebuild_routes(&mut inner);
    }

    pub async fn untrack_workspace(&self, target_cid: &str) -> Option<RunningWorkspace> {
        let mut inner = self.inner.write().await;
        let removed = inner.workspaces.remove(target_cid);
        if removed.is_some() {
            rebuild_routes(&mut inner);
        }
        removed
    }

    pub async fn http_route(&self, host: &str) -> Option<HostRoute> {
        let key = host.split(':').next().unwrap_or(host).to_lowercase();
        self.inner.read().await.http_routes.get(&key).cloned()
    }

    pub async fn tcp_route(&self, host_port: u16) -> Option<HostRoute> {
        self.inner.read().await.tcp_routes.get(&host_port).cloned()
    }

    /// Every non-80 host port across all currently-registered project configs.
    /// Used at startup to decide which TCP listeners to bind.
    pub async fn configured_tcp_ports(&self) -> Vec<u16> {
        let inner = self.inner.read().await;
        let mut ports: Vec<u16> = inner
            .configs
            .values()
            .flat_map(|c| c.services.iter())
            .flat_map(|s| s.ports.iter().map(|p| p.host))
            .filter(|p| *p != 80)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}

fn rebuild_routes(inner: &mut RegistryInner) {
    let mut http_routes = HashMap::new();
    let mut tcp_routes: HashMap<u16, HostRoute> = HashMap::new();
    for ws in inner.workspaces.values() {
        let Some(cfg) = inner.configs.get(&ws.project) else {
            continue;
        };
        // The workspace whose name matches the project name is the "root"
        // workspace and is rendered without the workspace label in front.
        let root = ws.workspace == cfg.project;
        for (hostname, route) in crate::routing::compute_http_routes(cfg, &ws.workspace, root) {
            http_routes.insert(hostname.to_lowercase(), route);
        }
        for (host_port, route) in crate::routing::compute_tcp_routes(cfg, &ws.workspace, root) {
            if let Some(existing) = tcp_routes.get(&host_port) {
                tracing::warn!(
                    host_port,
                    existing = %existing.socket_path,
                    new = %route.socket_path,
                    "tcp port already mapped; keeping existing route"
                );
                continue;
            }
            tcp_routes.insert(host_port, route);
        }
    }
    inner.http_routes = http_routes;
    inner.tcp_routes = tcp_routes;
}
