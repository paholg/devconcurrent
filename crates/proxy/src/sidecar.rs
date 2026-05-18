//! Lifecycle of the per-workspace socat sidecar.
//!
//! Mirrors the two-container forwarding pattern used by `dc fwd`. The sidecar
//! shares the devcontainer service container's network namespace and listens
//! on per-(service,port) unix sockets in a shared volume.

use docker::Docker;
use eyre::{Result, WrapErr};
use shared::{
    PROJECT_LABEL, PROXY_SIDECAR_LABEL, PROXY_SOCKS_DIR, PROXY_SOCKS_VOLUME, PROXY_TARGET_LABEL,
    ProjectProxyConfig, WORKSPACE_LABEL,
};

use crate::routing::sidecar_key;

pub const SOCAT_IMAGE: &str = "docker.io/alpine/socat:latest";

/// Create the inner sidecar joined to `target_cid`'s netns, listening on
/// per-`(service, container_port)` unix sockets. Returns the sidecar's container ID.
///
/// `cfg` describes the project (and all its services + ports); `target_cid` is
/// the devcontainer service container ID we net-join. The socket files live in
/// the shared `PROXY_SOCKS_VOLUME` mounted at `PROXY_SOCKS_DIR`.
pub async fn create_sidecar(
    docker: &Docker,
    cfg: &ProjectProxyConfig,
    workspace: &str,
    target_cid: &str,
) -> Result<String> {
    docker
        .ensure_image(SOCAT_IMAGE)
        .await
        .wrap_err("ensure socat image")?;

    let mut cmds: Vec<String> = Vec::new();
    for svc in &cfg.services {
        let target = if svc.name == cfg.devcontainer_service {
            "127.0.0.1".to_string()
        } else {
            svc.name.clone()
        };
        for port in &svc.ports {
            let key = sidecar_key(&cfg.project, workspace, &svc.name, port.container);
            // `unlink-early` removes any leftover socket file from a prior
            // sidecar before bind — without it socat exits with EADDRINUSE if
            // the volume already has the file.
            cmds.push(format!(
                "socat UNIX-LISTEN:{PROXY_SOCKS_DIR}/{key}.sock,unlink-early,fork,reuseaddr TCP:{target}:{}",
                port.container
            ));
        }
    }

    if cmds.is_empty() {
        eyre::bail!(
            "project {project:?} workspace {workspace:?} has no ports configured",
            project = cfg.project
        );
    }

    let shell_cmd = join_background(&cmds);
    let name = format!("devconcurrent-proxy-sidecar-{}-{workspace}", cfg.project);
    let name = sanitize_container_name(&name);
    let network_mode = format!("container:{target_cid}");
    let bind = format!("{PROXY_SOCKS_VOLUME}:{PROXY_SOCKS_DIR}");

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
        .with_bind(bind)
        .with_label(PROXY_SIDECAR_LABEL, "true")
        .with_label(PROXY_TARGET_LABEL, target_cid)
        .with_label(PROJECT_LABEL, &cfg.project)
        .with_label(WORKSPACE_LABEL, workspace)
        .call()
        .await
        .wrap_err("create sidecar container")?;
    docker
        .start_container(&id)
        .await
        .wrap_err("start sidecar container")?;
    Ok(id)
}

/// Remove sidecars by their container IDs. Errors are logged, not propagated;
/// best-effort cleanup is the right behavior here.
pub async fn remove_sidecar(docker: &Docker, id: &str) {
    match docker.remove_container(id).force(true).call().await {
        Ok(()) | Err(docker::Error::NotFound) => {}
        Err(e) => tracing::warn!(id = %id, "remove sidecar: {e}"),
    }
}

/// Find and remove every sidecar whose `PROXY_TARGET_LABEL` no longer points to
/// a running container — for example, leftovers after a proxy crash.
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
    let mut parts: Vec<String> = cmds.iter().map(|c| format!("{c} &")).collect();
    parts.push("wait".to_string());
    parts.join(" ")
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
