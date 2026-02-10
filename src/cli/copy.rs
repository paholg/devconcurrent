use std::borrow::Cow;

use bollard::Docker;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptionsBuilder, RemoveContainerOptions,
};
use clap::Args;
use eyre::eyre;
use futures::StreamExt;

use crate::cli::State;
use crate::run::{Runnable, Runner};
use crate::workspace::Workspace;

/// Copy named volumes from one workspace to another
#[derive(Debug, Args)]
#[command(verbatim_doc_comment)]
pub struct Copy {
    #[arg(short, long)]
    from: Option<String>,

    #[arg(short, long)]
    to: Option<String>,

    /// Volume names to copy [default: configured defaultCopyVolumes]
    volumes: Vec<String>,
}

impl Copy {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let from_ws = Workspace::get(&state, self.from.as_deref()).await?;
        let to_ws = Workspace::get(&state, self.to.as_deref()).await?;

        let volumes = if !self.volumes.is_empty() {
            self.volumes
        } else {
            let dc = state.devcontainer()?;
            dc.common
                .customizations
                .dc
                .default_copy_volumes
                .ok_or_else(|| eyre!("no volumes specified and no defaultCopyVolumes configured"))?
        };

        copy_volumes(
            &state.docker.docker,
            &volumes,
            &from_ws.compose_project_name,
            &to_ws.compose_project_name,
        )
        .await
    }
}

pub(crate) async fn copy_volumes(
    docker: &Docker,
    volumes: &[String],
    from_project: &str,
    to_project: &str,
) -> eyre::Result<()> {
    let copies = volumes.iter().map(|vol| CopyVolume {
        docker,
        name: vol.clone(),
        src: format!("{from_project}_{vol}"),
        dst: format!("{to_project}_{vol}"),
    });

    Runner::run_parallel("copy volumes", copies).await
}

struct CopyVolume<'a> {
    docker: &'a Docker,
    name: String,
    src: String,
    dst: String,
}

impl Runnable for CopyVolume<'_> {
    fn name(&self) -> Cow<'_, str> {
        (&self.name).into()
    }

    fn description(&self) -> Cow<'_, str> {
        format!("{} -> {}", self.src, self.dst).into()
    }

    async fn run(self, _: crate::run::Token) -> eyre::Result<()> {
        do_copy_volume(self.docker, &self.src, &self.dst).await
    }
}

const IMAGE: &str = "docker.io/library/alpine:latest";

async fn ensure_image(docker: &Docker) -> eyre::Result<()> {
    if docker.inspect_image(IMAGE).await.is_ok() {
        return Ok(());
    }
    docker
        .create_image(
            Some(CreateImageOptionsBuilder::new().from_image(IMAGE).build()),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;
    Ok(())
}

async fn do_copy_volume(docker: &Docker, src: &str, dst: &str) -> eyre::Result<()> {
    ensure_image(docker).await?;
    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: None,
                ..Default::default()
            }),
            ContainerCreateBody {
                image: Some(IMAGE.to_string()),
                cmd: Some(vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "cp -a /from/. /to/".to_string(),
                ]),
                host_config: Some(HostConfig {
                    binds: Some(vec![format!("{src}:/from"), format!("{dst}:/to")]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await?;

    let id = &container.id;
    let result = async {
        docker.start_container(id, None).await?;
        let mut stream = docker.wait_container(id, None);
        let resp = stream
            .next()
            .await
            .ok_or_else(|| eyre!("wait_container stream ended without response"))??;
        if resp.status_code != 0 {
            return Err(eyre!(
                "copy container exited with status {}",
                resp.status_code
            ));
        }
        Ok(())
    }
    .await;

    docker
        .remove_container(
            id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await?;

    result
}
