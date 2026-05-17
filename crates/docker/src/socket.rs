use std::env;
use std::path::PathBuf;

use tokio::process::Command;

use crate::error::{Error, Result};

/// Locate the Unix socket of the local docker (or podman-as-docker) daemon.
///
/// Resolution order (first existing socket wins; tried-and-missing paths are
/// included in the error if everything fails):
///
/// 1. `$DOCKER_HOST` env var, if set. Must use the `unix://` scheme.
/// 2. `docker context inspect`, if `docker` is on `PATH`.
/// 3. `$XDG_RUNTIME_DIR/podman/podman.sock` (rootless podman without docker CLI).
/// 4. `/var/run/docker.sock` and `/run/podman/podman.sock`, in that order.
pub async fn discover_socket() -> Result<PathBuf> {
    let mut tried = Vec::new();

    if let Ok(host) = env::var("DOCKER_HOST") {
        let raw = host
            .strip_prefix("unix://")
            .ok_or_else(|| Error::NonUnixHost { host: host.clone() })?;
        let socket = PathBuf::from(raw);
        if socket.exists() {
            return Ok(socket);
        }
        tried.push(socket);
    }

    if let Some(socket) = docker_context_socket().await {
        if socket.exists() {
            return Ok(socket);
        }
        tried.push(socket);
    }

    if let Ok(xdg) = env::var("XDG_RUNTIME_DIR") {
        let socket = PathBuf::from(xdg).join("podman/podman.sock");
        if socket.exists() {
            return Ok(socket);
        }
        tried.push(socket);
    }

    for path in ["/var/run/docker.sock", "/run/podman/podman.sock"] {
        let socket = PathBuf::from(path);
        if socket.exists() {
            return Ok(socket);
        }
        tried.push(socket);
    }

    Err(Error::SocketNotFound { tried })
}

async fn docker_context_socket() -> Option<PathBuf> {
    let out = Command::new("docker")
        .args(["context", "inspect", "-f", "{{.Endpoints.docker.Host}}"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let host = String::from_utf8(out.stdout).ok()?;
    host.trim().strip_prefix("unix://").map(PathBuf::from)
}
