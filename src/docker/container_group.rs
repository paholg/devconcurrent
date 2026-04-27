use std::{collections::HashMap, path::PathBuf};

use futures::future::try_join_all;

use crate::{
    docker::ContainerInfo,
    state::{DevcontainerState, State},
    workspace::{Workspace, WorkspaceLegacy, git_status},
    worktree,
};

/// Group of containers by worktree path
pub(crate) struct ContainerGroup {
    pub(crate) path: PathBuf,
    pub(crate) containers: Vec<ContainerInfo>,
}

impl ContainerGroup {
    pub(crate) async fn list(
        state: &State,
        devcontainer: &DevcontainerState,
    ) -> eyre::Result<(Vec<Self>, HashMap<String, Vec<u16>>)> {
        let worktree_paths = worktree::list(&state.project.path).await?;
        let (containers, fwd_ports) = tokio::try_join!(
            devcontainer.docker.container_info(),
            devcontainer.docker.forwarded_ports(&state.project_name),
        )?;

        let mut groups: HashMap<PathBuf, ContainerGroup> = HashMap::new();
        for c in containers {
            if c.dc_project
                .as_ref()
                .is_some_and(|p| p != &state.project_name)
            {
                // This is a dc-managed container for a different project.
                continue;
            }
            if c.dc_project.is_none() && !worktree_paths.contains(&c.local_folder) {
                // This is not a devcontainer for any of our worktrees.
                continue;
            }
            let group = groups
                .entry(c.local_folder.clone())
                .or_insert_with(|| ContainerGroup {
                    path: c.local_folder.clone(),
                    containers: Vec::new(),
                });
            group.containers.push(c);
        }

        for path in worktree_paths {
            groups
                .entry(path.clone())
                .or_insert_with(|| ContainerGroup {
                    path,
                    containers: Vec::new(),
                });
        }
        Ok((groups.into_values().collect(), fwd_ports))
    }

    pub(crate) async fn into_workspace(
        self,
        state: &State,
        devcontainer: &DevcontainerState,
        fwd_ports: &HashMap<String, Vec<u16>>,
    ) -> eyre::Result<WorkspaceLegacy> {
        let git_future = git_status::GitStatus::fetch(&self.path);
        let execs_futures = try_join_all(
            self.containers
                .iter()
                .map(|c| devcontainer.docker.execs(&c.id)),
        );
        let stats_futures = try_join_all(
            self.containers
                .iter()
                .map(|c| devcontainer.docker.stats(&c.id)),
        );
        let (git_status, execs, stats) =
            tokio::try_join!(git_future, execs_futures, stats_futures)?;

        let execs = execs.into_iter().sum();
        let stats = stats.into_iter().sum();

        let root = self.path == state.project.path;
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let ws_mini = Workspace::from_path(self.path.clone(), state)
            .ok_or_else(|| eyre::eyre!("Invalid path: {}", self.path.display()))?;
        let mut fwd_ports = fwd_ports.get(&ws_mini.name).cloned().unwrap_or_default();
        fwd_ports.sort();
        fwd_ports.dedup();

        let mut docker_ports: Vec<u16> = self
            .containers
            .iter()
            .flat_map(|c| &c.host_ports)
            .copied()
            .collect();
        docker_ports.sort();
        docker_ports.dedup();

        let dc_managed = self.containers.iter().any(|c| c.dc_project.is_some());

        Ok(WorkspaceLegacy {
            name,
            root,
            containers: self.containers,
            git_status,
            execs,
            stats,
            fwd_ports,
            docker_ports,
            dc_managed,
        })
    }
}
