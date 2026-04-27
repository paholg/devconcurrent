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
}

pub(crate) struct WorkspaceLegacy {
    pub(crate) name: String,
    pub(crate) root: bool,
    pub(crate) compose_project_name: String,
    pub(crate) containers: Vec<ContainerInfo>,
    pub(crate) git_status: git_status::GitStatus,
    pub(crate) execs: usize,
    pub(crate) stats: Stats,
    pub(crate) fwd_ports: Vec<u16>,
    pub(crate) docker_ports: Vec<u16>,
    pub(crate) dc_managed: bool,
}

impl WorkspaceLegacy {
    pub(crate) async fn get(
        state: &State,
        devcontainer: &DevcontainerState,
        name: &str,
    ) -> eyre::Result<WorkspaceLegacy> {
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
    ) -> eyre::Result<WorkspaceLegacy> {
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

    pub(crate) fn is_dirty(&self) -> bool {
        self.git_status.is_dirty()
    }

    pub(crate) fn service_container_id(&self) -> eyre::Result<&str> {
        // FIXME: We need to find the correct service container.
        Ok(&self
            .containers
            .first()
            .ok_or_else(|| eyre!("no containers for workspace"))?
            .id)
    }
}
