//! Lifecycle of the per-service socat sidecar.
//!
//! One sidecar per `(workspace, service)` that has port remappings, joined to
//! that service's container network namespace. The sidecar binds each
//! configured `host` port inside that netns and forwards locally to
//! `127.0.0.1:<container>` — which inside the service's netns is the service
//! itself.
//!
//! Clients reach the service by connecting to that container's IP at
//! `<host>` — on Linux directly via the docker bridge, on macOS via a tunnel
//! such as docker-mac-net-connect.

use docker::Docker;
use eyre::{Result, WrapErr};
use shared::{
    PROJECT_LABEL, PROXY_SIDECAR_LABEL, PROXY_TARGET_LABEL, ServiceConfig, WORKSPACE_LABEL,
};

pub const SOCAT_IMAGE: &str = "docker.io/alpine/socat:latest";

/// Create the sidecar joined to `target_cid`'s netns and start it. Returns
/// the sidecar's container ID, or `None` if `svc` has no port mappings.
pub async fn create_sidecar(
    docker: &Docker,
    project: &str,
    workspace: &str,
    svc: &ServiceConfig,
    target_cid: &str,
) -> Result<Option<String>> {
    if svc.ports.is_empty() {
        return Ok(None);
    }

    docker
        .ensure_image(SOCAT_IMAGE)
        .await
        .wrap_err("ensure socat image")?;

    let cmds: Vec<String> = svc
        .ports
        .iter()
        .map(|p| {
            format!(
                "socat TCP-LISTEN:{host},fork,reuseaddr TCP:127.0.0.1:{container}",
                host = p.host,
                container = p.container,
            )
        })
        .collect();
    let shell_cmd = join_background(&cmds);
    let name = sanitize_container_name(&format!(
        "devconcurrent-proxy-sidecar-{project}-{workspace}-{}",
        svc.name
    ));
    let network_mode = format!("container:{target_cid}");

    // If a stale sidecar exists from a previous run, force-remove it first.
    match docker.remove_container(&name).force(true).call().await {
        Ok(()) | Err(docker::Error::NotFound) => {}
        Err(e) => {
            tracing::warn!(name = %name, "remove stale sidecar: {e}");
        }
    }

    let id = docker
        .create_container(&name)
        .image(SOCAT_IMAGE)
        .network_mode(&network_mode)
        .entrypoint(vec!["sh".to_string()])
        .cmd(vec!["-c".to_string(), shell_cmd])
        .with_label(PROXY_SIDECAR_LABEL, "true")
        .with_label(PROXY_TARGET_LABEL, target_cid)
        .with_label(PROJECT_LABEL, project)
        .with_label(WORKSPACE_LABEL, workspace)
        .call()
        .await
        .wrap_err("create sidecar container")?;
    docker
        .start_container(&id)
        .await
        .wrap_err("start sidecar container")?;
    Ok(Some(id))
}

/// Remove a sidecar by its container ID. Errors are logged, not propagated.
pub async fn remove_sidecar(docker: &Docker, id: &str) {
    match docker.remove_container(id).force(true).call().await {
        Ok(()) | Err(docker::Error::NotFound) => {}
        Err(e) => tracing::warn!(id = %id, "remove sidecar: {e}"),
    }
}

/// Remove every sidecar whose `PROXY_TARGET_LABEL` no longer points to a
/// running container — leftovers after a proxy crash.
pub async fn sweep_orphans(docker: &Docker) -> Result<()> {
    let sidecars = docker
        .list_containers()
        .all(true)
        .with_label(PROXY_SIDECAR_LABEL, "true")
        .call()
        .await
        .wrap_err("list sidecars")?;
    for sc in sidecars {
        let target = sc.labels.get(PROXY_TARGET_LABEL).cloned();
        let alive = match target {
            Some(cid) => match docker.inspect_container(&cid).await {
                Ok(d) => d.state.running,
                Err(docker::Error::NotFound) => false,
                Err(e) => {
                    tracing::warn!(cid = %cid, "inspect target during sweep: {e}");
                    true
                }
            },
            None => false,
        };
        if !alive {
            tracing::info!(sidecar = %sc.id, "removing orphaned sidecar");
            remove_sidecar(docker, &sc.id).await;
        }
    }
    Ok(())
}

fn join_background(cmds: &[String]) -> String {
    // `set -e` + a trap kills the whole sidecar if any single socat dies, so
    // failures aren't silently masked. The sidecar then restarts via its
    // docker lifecycle (or the proxy re-creates it on the next start event).
    let mut parts = vec!["set -e".to_string(), "trap 'kill 0' CHLD".to_string()];
    for c in cmds {
        parts.push(format!("{c} &"));
    }
    parts.push("wait".to_string());
    parts.join("; ")
}

fn sanitize_container_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
