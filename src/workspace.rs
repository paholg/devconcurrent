use std::path::PathBuf;

use bollard::models::ContainerSummaryStateEnum;
use eyre::eyre;
use futures::future::try_join_all;

use crate::cli::State;
use crate::docker::container_group::ContainerGroup;
use crate::docker::{ContainerInfo, ExecSession, Stats};

pub mod git_status;
pub mod table;

#[derive(Debug)]
pub struct Workspace {
    pub path: PathBuf,
    pub name: String,
    pub root: bool,
    pub compose_project_name: String,
    pub containers: Vec<ContainerInfo>,
    pub git_status: git_status::GitStatus,
    pub execs: Vec<ExecSession>,
    pub stats: Stats,
    pub fwd_ports: Vec<u16>,
    pub docker_ports: Vec<u16>,
    pub dc_managed: bool,
}

impl Workspace {
    pub async fn get(state: &State, name: &str) -> eyre::Result<Workspace> {
        Self::get_inner(state, name, ContainerGroup::list(state).await?).await
    }

    pub async fn get_including_archived(state: &State, name: &str) -> eyre::Result<Workspace> {
        Self::get_inner(
            state,
            name,
            ContainerGroup::list_including_archived(state).await?,
        )
        .await
    }

    async fn get_inner(
        state: &State,
        name: &str,
        (groups, fwd_ports): (
            Vec<ContainerGroup>,
            std::collections::HashMap<String, Vec<u16>>,
        ),
    ) -> eyre::Result<Workspace> {
        let group = if state.is_root(name) {
            groups
                .into_iter()
                .find(|g| g.path == state.project.path)
                .ok_or_else(|| eyre!("root workspace not found"))?
        } else {
            groups
                .into_iter()
                .find(|g| g.path.file_name().is_some_and(|f| f == name))
                .ok_or_else(|| eyre!("no workspace found for name {name}"))?
        };
        group.into_workspace(state, &fwd_ports).await
    }

    pub async fn list(state: &State) -> eyre::Result<Vec<Workspace>> {
        let (groups, fwd_ports) = ContainerGroup::list(state).await?;
        let futures = groups
            .into_iter()
            .map(|g| g.into_workspace(state, &fwd_ports));

        try_join_all(futures).await
    }

    pub fn status(&self) -> ContainerSummaryStateEnum {
        self.containers
            .iter()
            .map(|c| c.state)
            .max()
            .unwrap_or(ContainerSummaryStateEnum::EMPTY)
    }

    pub fn created(&self) -> Option<i64> {
        self.containers.iter().filter_map(|c| c.created).min()
    }

    pub fn is_dirty(&self) -> bool {
        self.git_status.is_dirty()
    }

    pub fn service_container_id(&self) -> eyre::Result<&str> {
        // FIXME: We need to find the correct service container.
        Ok(&self
            .containers
            .first()
            .ok_or_else(|| eyre!("no containers for workspace"))?
            .id)
    }
}
