use std::path::PathBuf;

use bollard::models::ContainerSummaryStateEnum;
use eyre::eyre;
use futures::future::try_join_all;

use crate::docker::container_group::ContainerGroup;
use crate::docker::{ContainerInfo, ExecSession, Stats};
use crate::state::{DevcontainerState, State};

pub mod git_status;
pub mod table;

#[derive(Debug)]
pub struct WorkspaceMini {
    pub name: String,
    pub path: PathBuf,
    pub root: bool,
}

impl WorkspaceMini {
    pub fn from_path(path: PathBuf, state: &State) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_string();
        let root = state.is_root(&name);

        Some(Self { name, path, root })
    }

    /// Match the devcontainer CLI convention: `{basename}_devcontainer`, lowercased,
    /// keeping only `[a-z0-9-_]`.
    pub fn compose_project_name(&self) -> String {
        let raw = format!("{}_devcontainer", self.name);

        raw.to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect()
    }

    pub fn docker_labels(&self, state: &State) -> Vec<String> {
        vec![
            format!("dev.devconcurrent.project={}", state.project_name),
            format!("dev.devconcurrent.workspace={}", self.name),
        ]
    }
}

#[derive(Debug)]
pub struct Workspace {
    pub name: String,
    pub path: PathBuf,
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
    pub async fn get(
        state: &State,
        devcontainer: &DevcontainerState,
        name: &str,
    ) -> eyre::Result<Workspace> {
        Self::get_inner(
            state,
            devcontainer,
            name,
            ContainerGroup::list(state, devcontainer).await?,
        )
        .await
    }

    async fn get_inner(
        state: &State,
        devcontainer: &DevcontainerState,
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
        group.into_workspace(state, devcontainer, &fwd_ports).await
    }

    pub async fn list(
        state: &State,
        devcontainer: &DevcontainerState,
    ) -> eyre::Result<Vec<Workspace>> {
        let (groups, fwd_ports) = ContainerGroup::list(state, devcontainer).await?;
        let futures = groups
            .into_iter()
            .map(|g| g.into_workspace(state, devcontainer, &fwd_ports));

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
