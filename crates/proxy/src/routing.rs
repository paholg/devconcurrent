//! Hostname template rendering + sidecar key derivation.

use handlebars::Handlebars;
use serde::Serialize;
use shared::{PROXY_SOCKS_DIR, ProjectProxyConfig};

use crate::registry::HostRoute;

#[derive(Serialize)]
struct TemplateContext<'a> {
    root: bool,
    project: &'a str,
    workspace: &'a str,
    service: &'a str,
}

/// Render the hostname for one (project, workspace, service) tuple using the
/// project's domain template. Logs and returns `None` if the template fails.
pub fn render_hostname(
    cfg: &ProjectProxyConfig,
    workspace: &str,
    service: &str,
    root: bool,
) -> Option<String> {
    let mut hbs = Handlebars::new();
    hbs.set_strict_mode(false);
    let ctx = TemplateContext {
        root,
        project: &cfg.project,
        workspace,
        service,
    };
    match hbs.render_template(&cfg.domain_template, &ctx) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(
                project = %cfg.project,
                template = %cfg.domain_template,
                "failed to render domain template: {e}"
            );
            None
        }
    }
}

/// Sidecar key for `(project, workspace, service, container_port)`. Used as
/// both the unix-socket filename and the routing identifier. Sanitized so it's
/// filename-safe on every OS.
pub fn sidecar_key(project: &str, workspace: &str, service: &str, container_port: u16) -> String {
    let raw = format!("{project}-{workspace}-{service}-{container_port}");
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Path to a sidecar's unix socket inside the proxy container.
pub fn socket_path(key: &str) -> String {
    format!("{PROXY_SOCKS_DIR}/{key}.sock")
}

/// Compute all `(hostname, HostRoute)` pairs for a workspace's port-80 services.
pub fn compute_http_routes(
    cfg: &ProjectProxyConfig,
    workspace: &str,
    root: bool,
) -> Vec<(String, HostRoute)> {
    let mut out = Vec::new();
    for svc in &cfg.services {
        let Some(hostname) = render_hostname(cfg, workspace, &svc.name, root) else {
            continue;
        };
        if let Some(port_80) = svc.ports.iter().find(|p| p.host == 80) {
            let key = sidecar_key(&cfg.project, workspace, &svc.name, port_80.container);
            out.push((
                hostname,
                HostRoute {
                    socket_path: socket_path(&key),
                },
            ));
        }
    }
    out
}

/// Compute all `(host_port, HostRoute)` pairs for a workspace's non-port-80 services.
pub fn compute_tcp_routes(
    cfg: &ProjectProxyConfig,
    workspace: &str,
    _root: bool,
) -> Vec<(u16, HostRoute)> {
    let mut out = Vec::new();
    for svc in &cfg.services {
        for port in &svc.ports {
            if port.host == 80 {
                continue;
            }
            let key = sidecar_key(&cfg.project, workspace, &svc.name, port.container);
            out.push((
                port.host,
                HostRoute {
                    socket_path: socket_path(&key),
                },
            ));
        }
    }
    out
}
