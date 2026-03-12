use std::{collections::HashMap, path::PathBuf};

use futures::future::try_join_all;

use crate::{
    archive,
    cli::State,
    docker::{ContainerInfo, compose::compose_project_name},
    workspace::{Workspace, git_status},
    worktree,
};

/// Group of containers by worktree path
pub struct ContainerGroup {
    pub path: PathBuf,
    pub containers: Vec<ContainerInfo>,
}

impl ContainerGroup {
    pub async fn list(state: &State) -> eyre::Result<(Vec<Self>, HashMap<String, Vec<u16>>)> {
        Self::list_inner(state, false).await
    }

    /// Like `list`, but includes archived workspaces.
    pub async fn list_including_archived(
        state: &State,
    ) -> eyre::Result<(Vec<Self>, HashMap<String, Vec<u16>>)> {
        Self::list_inner(state, true).await
    }

    async fn list_inner(
        state: &State,
        include_archived: bool,
    ) -> eyre::Result<(Vec<Self>, HashMap<String, Vec<u16>>)> {
        let worktree_paths = worktree::list(&state.project.path).await?;
        let (containers, fwd_ports) = tokio::try_join!(
            state.docker.container_info(),
            state.docker.forwarded_ports(&state.project_name),
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
            if !include_archived {
                let cp = compose_project_name(&c.local_folder);
                if archive::is_archived(&state.project_name, &cp) {
                    continue;
                }
            }
            let group = groups
                .entry(c.local_folder.clone())
                .or_insert_with(|| ContainerGroup {
                    path: c.local_folder.clone(),
                    containers: Vec::new(),
                });
            group.containers.push(c);
        }

        // Ensure we have an entry for all of our worktrees (skip archived ones).
        for path in worktree_paths {
            if !include_archived {
                let cp = compose_project_name(&path);
                if archive::is_archived(&state.project_name, &cp) {
                    continue;
                }
            }
            groups
                .entry(path.clone())
                .or_insert_with(|| ContainerGroup {
                    path,
                    containers: Vec::new(),
                });
        }
        Ok((groups.into_values().collect(), fwd_ports))
    }

    pub async fn into_workspace(
        self,
        state: &State,
        fwd_ports: &HashMap<String, Vec<u16>>,
    ) -> eyre::Result<Workspace> {
        let git_future = git_status::GitStatus::fetch(&self.path);
        let execs_futures = try_join_all(self.containers.iter().map(|c| state.docker.execs(&c.id)));
        let stats_futures = try_join_all(self.containers.iter().map(|c| state.docker.stats(&c.id)));
        let (git_status, execs, stats) =
            tokio::try_join!(git_future, execs_futures, stats_futures)?;

        let execs = execs.into_iter().flatten().collect();
        let stats = stats.into_iter().sum();

        let root = self.path == state.project.path;
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let compose_project_name = compose_project_name(&self.path);
        let mut fwd_ports = fwd_ports
            .get(&compose_project_name)
            .cloned()
            .unwrap_or_default();
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

        Ok(Workspace {
            compose_project_name,
            path: self.path,
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
