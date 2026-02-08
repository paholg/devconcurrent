use std::process::Stdio;

use bollard::Docker;
use eyre::bail;
use tokio::process::Command;

use crate::docker;

pub async fn check() -> eyre::Result<Docker> {
    let docker = match docker::client() {
        Ok(d) => d,
        Err(_) => {
            bail!(
                "docker is not installed or the daemon is not running.\nInstall Docker: https://docs.docker.com/get-docker/"
            );
        }
    };

    if docker.version().await.is_err() {
        bail!(
            "docker is not installed or the daemon is not running.\nInstall Docker: https://docs.docker.com/get-docker/"
        );
    }

    if Command::new("docker")
        .args(["compose", "version", "--short"])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .await
        .map_or(true, |s| !s.success())
    {
        bail!(
            "docker compose (v2) is not available.\nInstall the Compose plugin: https://docs.docker.com/compose/install/"
        );
    }

    if Command::new("docker")
        .args(["buildx", "version"])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .await
        .map_or(true, |s| !s.success())
    {
        bail!(
            "docker buildx is not available.\nInstall the Buildx plugin: https://docs.docker.com/build/install-buildx/"
        );
    }

    Ok(docker)
}
