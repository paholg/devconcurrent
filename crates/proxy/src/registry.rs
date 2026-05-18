//! Shared in-memory state: pushed project configs + currently-tracked service
//! containers.
//!
//! The proxy reads `/etc/projects/*.json` from a docker volume on startup, and
//! mutates the service map in response to docker container start/die events.
//! The derived `names` map (hostname → container IP) is rebuilt on each
//! change and consumed by the DNS server.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use shared::ProjectProxyConfig;
use tokio::sync::RwLock;

/// One running compose service container tracked from docker start events.
#[derive(Debug, Clone)]
pub struct RunningService {
    pub project: String,
    pub workspace: String,
    pub service: String,
    pub target_cid: String,
    pub container_ip: IpAddr,
    pub sidecar_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct RegistryInner {
    pub configs: HashMap<String, ProjectProxyConfig>,
    pub services: HashMap<String, RunningService>,
    pub names: HashMap<String, IpAddr>,
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
        rebuild_names(&mut inner);
    }

    pub async fn config_for(&self, project: &str) -> Option<ProjectProxyConfig> {
        self.inner.read().await.configs.get(project).cloned()
    }

    pub async fn track_service(&self, svc: RunningService) {
        let mut inner = self.inner.write().await;
        inner.services.insert(svc.target_cid.clone(), svc);
        rebuild_names(&mut inner);
    }

    pub async fn has_service(&self, target_cid: &str) -> bool {
        self.inner.read().await.services.contains_key(target_cid)
    }

    pub async fn untrack_service(&self, target_cid: &str) -> Option<RunningService> {
        let mut inner = self.inner.write().await;
        let removed = inner.services.remove(target_cid);
        if removed.is_some() {
            rebuild_names(&mut inner);
        }
        removed
    }

    /// Lookup a hostname → IP for DNS. The caller is expected to have already
    /// lowercased the host and trimmed any trailing dot.
    pub async fn resolve(&self, host: &str) -> Option<IpAddr> {
        self.inner.read().await.names.get(host).copied()
    }
}

fn rebuild_names(inner: &mut RegistryInner) {
    let mut names: HashMap<String, IpAddr> = HashMap::new();
    for svc in inner.services.values() {
        let Some(cfg) = inner.configs.get(&svc.project) else {
            continue;
        };
        let root = svc.workspace == cfg.project;
        let Some(hostname) =
            crate::routing::render_hostname(cfg, &svc.workspace, &svc.service, root)
        else {
            continue;
        };
        let key = hostname.to_lowercase();
        if let Some(existing) = names.get(&key) {
            if *existing != svc.container_ip {
                tracing::warn!(
                    hostname = %key,
                    existing = %existing,
                    new = %svc.container_ip,
                    "hostname collision; keeping existing"
                );
            }
            continue;
        }
        names.insert(key, svc.container_ip);
    }
    inner.names = names;
}
