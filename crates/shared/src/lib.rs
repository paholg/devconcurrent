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
/// On the proxy container only.
pub const PROXY_LABEL: &str = "dev.devconcurrent.proxy";
/// On sidecars only.
pub const PROXY_SIDECAR_LABEL: &str = "dev.devconcurrent.proxy.sidecar";
/// On both the proxy container and every sidecar — the "everything the
/// proxy owns" umbrella. Lets `dc proxy up` / `dc proxy down` do one
/// `list_containers` to find the whole group.
pub const PROXY_GROUP_LABEL: &str = "dev.devconcurrent.proxy.group";
/// Present on sidecars only. Value is the container id of the service the
/// sidecar is net-joined to.
pub const PROXY_TARGET_LABEL: &str = "dev.devconcurrent.proxy.target";
pub const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";
pub const COMPOSE_SERVICE_LABEL: &str = "com.docker.compose.service";

// Resource names.
pub const PROXY_CONTAINER_NAME: &str = "devconcurrent-proxy";
pub const PROXY_CONFIG_VOLUME: &str = "devconcurrent-proxy-config";

// In-container paths.
pub const PROXY_CONFIG_DIR: &str = "/etc/projects";
/// Directory inside the proxy container where the mkcert CAROOT is
/// bind-mounted read-only when TLS is enabled.
pub const PROXY_CA_DIR: &str = "/etc/proxy-ca";
/// Directory inside each sidecar container where the proxy writes the per-
/// service plan and (if TLS is enabled) cert + key.
pub const SIDECAR_PLAN_DIR: &str = "/etc/sidecar";
pub const SIDECAR_PLAN_FILE: &str = "plan.json";
pub const SIDECAR_CERT_FILE: &str = "cert.pem";
pub const SIDECAR_KEY_FILE: &str = "key.pem";

// Environment variables read by the proxy on startup.
pub const ENV_DNS_PORT: &str = "DC_PROXY_DNS_PORT";
/// Set by the CLI when a CAROOT bind-mount is present. The proxy loads
/// `rootCA.pem` + `rootCA-key.pem` from this directory.
pub const ENV_CA_DIR: &str = "DC_PROXY_CA_DIR";

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
    /// Terminate TLS on `host` and forward plaintext to `container`. Requires
    /// the proxy to have a CAROOT mounted; otherwise the sidecar logs a
    /// warning and leaves this port unbound.
    #[serde(default)]
    pub tls: bool,
}

/// Sidecar plan, written by the proxy into the sidecar container's
/// filesystem at `<SIDECAR_PLAN_DIR>/<SIDECAR_PLAN_FILE>` before start.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarPlan {
    /// Rendered hostname for this service; used as the TLS cert's SAN.
    pub hostname: String,
    pub ports: Vec<PortMapping>,
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
                    tls: false,
                }],
            }],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ProjectProxyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.project, "p");
        assert_eq!(back.services[0].ports[0].host, 80);
    }
}
