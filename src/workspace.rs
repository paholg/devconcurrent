use std::path::PathBuf;

use bollard::models::ContainerSummaryStateEnum;
use eyre::eyre;
use futures::future::try_join_all;

use crate::docker::container_group::ContainerGroup;
use crate::docker::{ContainerInfo, Stats};
use crate::state::{DevcontainerState, State};

pub(crate) mod git_status;
pub(crate) mod table;

pub(crate) struct Workspace<'a> {
    pub(crate) state: &'a State,
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_root: bool,
}

impl<'a> Workspace<'a> {
    pub(crate) fn from_path(path: PathBuf, state: &'a State) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_string();
        let is_root = state.is_root(&name);

        Some(Self {
            state,
            name,
            path,
            is_root,
        })
    }

    pub(crate) async fn is_dirty(&self) -> eyre::Result<bool> {
        Ok(git_status::GitStatus::fetch(&self.path).await?.is_dirty())
    }

    /// Match the devcontainer CLI convention: `{basename}_devcontainer`, lowercased,
    /// keeping only `[a-z0-9-_]`.
    pub(crate) fn compose_project_name(&self) -> String {
        let raw = format!("{}_devcontainer", self.name);

        raw.to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect()
    }

    pub(crate) fn project_label(&self) -> String {
        format!("dev.devconcurrent.project={}", self.state.project_name)
    }

    pub(crate) fn workspace_label(&self) -> String {
        format!("dev.devconcurrent.workspace={}", self.name)
    }

    pub(crate) fn fwd_label(&self) -> String {
        "dev.devconcurrent.fwd=true".to_string()
    }

    pub(crate) fn docker_labels(&self) -> Vec<String> {
        vec![self.project_label(), self.workspace_label()]
    }

    pub(crate) fn docker_fwd_labels(&self) -> Vec<String> {
        vec![
            self.project_label(),
            self.workspace_label(),
            self.fwd_label(),
        ]
    }

    pub(crate) async fn devcontainer(
        &'a self,
        devcontainer: &'a DevcontainerState,
    ) -> eyre::Result<WorkspaceDevcontainer<'a>> {
        let containers = devcontainer.docker.workspace_container_info(self).await?;
        Ok(WorkspaceDevcontainer {
            devcontainer,
            containers,
        })
    }
}

pub(crate) struct WorkspaceDevcontainer<'a> {
    devcontainer: &'a DevcontainerState,
    containers: Vec<ContainerInfo>,
}

impl<'a> WorkspaceDevcontainer<'a> {
    pub(crate) fn status(&self) -> ContainerSummaryStateEnum {
        self.containers
            .iter()
            .map(|c| c.state)
            .max()
            .unwrap_or(ContainerSummaryStateEnum::EMPTY)
    }

    pub(crate) fn service_container_id(&self) -> eyre::Result<&str> {
        // FIXME: We need to find the correct service container.
        Ok(&self
            .containers
            .first()
            .ok_or_else(|| eyre!("no containers for workspace"))?
            .id)
    }

    pub(crate) async fn execs(&self) -> eyre::Result<usize> {
        let counts = try_join_all(
            self.containers
                .iter()
                .map(|c| self.devcontainer.docker.execs(&c.id)),
        )
        .await?;
        Ok(counts.into_iter().sum())
    }
}

pub(crate) struct WorkspaceLegacy {
    pub(crate) name: String,
    pub(crate) root: bool,
    pub(crate) containers: Vec<ContainerInfo>,
    pub(crate) git_status: git_status::GitStatus,
    pub(crate) execs: usize,
    pub(crate) stats: Stats,
    pub(crate) fwd_ports: Vec<u16>,
    pub(crate) docker_ports: Vec<u16>,
    pub(crate) dc_managed: bool,
}

impl WorkspaceLegacy {
    pub(crate) async fn list(
        state: &State,
        devcontainer: &DevcontainerState,
    ) -> eyre::Result<Vec<WorkspaceLegacy>> {
        let (groups, fwd_ports) = ContainerGroup::list(state, devcontainer).await?;
        let futures = groups
            .into_iter()
            .map(|g| g.into_workspace(state, devcontainer, &fwd_ports));

        try_join_all(futures).await
    }

    pub(crate) fn status(&self) -> ContainerSummaryStateEnum {
        self.containers
            .iter()
            .map(|c| c.state)
            .max()
            .unwrap_or(ContainerSummaryStateEnum::EMPTY)
    }

    pub(crate) fn created(&self) -> Option<i64> {
        self.containers.iter().filter_map(|c| c.created).min()
    }
}
