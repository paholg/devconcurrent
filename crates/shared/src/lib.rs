//! Wire format and shared constants between the devconcurrent CLI and the
//! devconcurrent-proxy service.
//!
//! The CLI writes one `<project>.json` file per project into the
//! `devconcurrent-proxy-config` volume; the proxy reads them at startup.

use serde::{Deserialize, Serialize};

// Container labels.
pub const PROJECT_LABEL: &str = "dev.devconcurrent.project";
pub const WORKSPACE_LABEL: &str = "dev.devconcurrent.workspace";
pub const MANAGED_LABEL: &str = "dev.devconcurrent.managed";
/// Marks the global proxy container.
pub const PROXY_LABEL: &str = "dev.devconcurrent.proxy";
/// Marks a per-workspace socat sidecar created by the proxy.
pub const PROXY_SIDECAR_LABEL: &str = "dev.devconcurrent.proxy.sidecar";
pub const PROXY_TARGET_LABEL: &str = "dev.devconcurrent.proxy.target";
/// Hex sha256 of the proxy container's stable input config (image, binds, env,
/// network_mode). Lets the CLI detect a stale container whose binds/env have
/// drifted from what the current CLI would create and recreate it.
pub const PROXY_CONFIG_HASH_LABEL: &str = "dev.devconcurrent.proxy.config-hash";
pub const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";
pub const COMPOSE_SERVICE_LABEL: &str = "com.docker.compose.service";

// Resource names.
pub const PROXY_CONTAINER_NAME: &str = "devconcurrent-proxy";
pub const PROXY_CONFIG_VOLUME: &str = "devconcurrent-proxy-config";

// In-container paths.
pub const PROXY_CONFIG_DIR: &str = "/etc/projects";

// Environment variables read by the proxy on startup.
pub const ENV_DNS_PORT: &str = "DC_PROXY_DNS_PORT";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProxyConfig {
    pub project: String,
    /// Handlebars source. Variables: `root` (bool), `project`, `workspace`, `service`.
    pub domain_template: String,
    pub services: Vec<ServiceConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConfig {
    /// Compose service name. The sidecar runs in this service's container
    /// netns and forwards `host` → `127.0.0.1:container` locally.
    pub name: String,
    pub ports: Vec<PortMapping>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    /// Port the sidecar listens on, in the devcontainer service's netns.
    pub host: u16,
    /// Destination port inside the target service container.
    pub container: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cfg = ProjectProxyConfig {
            project: "p".into(),
            domain_template: "{{service}}.{{project}}.test".into(),
            services: vec![ServiceConfig {
                name: "app".into(),
                ports: vec![PortMapping {
                    host: 80,
                    container: 3000,
                }],
            }],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ProjectProxyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.project, "p");
        assert_eq!(back.services[0].ports[0].host, 80);
    }
}
