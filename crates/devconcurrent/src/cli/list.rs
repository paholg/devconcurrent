use std::collections::HashMap;

use clap::Args;
use futures::future::try_join_all;

use crate::{
    cli::State,
    state::DevcontainerState,
    workspace::{
        Workspace,
        git_status::GitStatus,
        table::{DockerCells, WorkspaceListRow, workspace_table},
    },
};

/// List all workspaces for the project
#[derive(Debug, Args)]
pub(crate) struct List;

impl List {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        let workspaces = Workspace::list(&state).await?;
        let rows = if let Some(dc) = state.devcontainer.as_ref() {
            let fwd_ports_map = dc.docker.forwarded_ports(&state.project_name).await?;

            try_join_all(
                workspaces
                    .iter()
                    .map(|ws| build_devcontainer_row(ws, dc, &fwd_ports_map)),
            )
            .await?
        } else {
            try_join_all(workspaces.iter().map(|ws| build_compact_row(ws))).await?
        };

        eprint!("{}", workspace_table(&rows));
        Ok(())
    }
}
async fn build_compact_row(ws: &Workspace<'_>) -> eyre::Result<WorkspaceListRow> {
    Ok(WorkspaceListRow {
        name: ws.name.clone(),
        is_root: ws.is_root,
        git_status: GitStatus::fetch(&ws.path).await?,
        docker: None,
    })
}

async fn build_devcontainer_row(
    ws: &Workspace<'_>,
    dc: &DevcontainerState,
    fwd_ports_map: &HashMap<String, Vec<u16>>,
) -> eyre::Result<WorkspaceListRow> {
    let git_future = GitStatus::fetch(&ws.path);

    let wsdc = ws.devcontainer(dc).await?;
    let (git_status, stats, execs) = tokio::try_join!(git_future, wsdc.stats(), wsdc.execs())?;

    let mut fwd_ports = fwd_ports_map.get(&ws.name).cloned().unwrap_or_default();
    fwd_ports.sort();
    fwd_ports.dedup();

    Ok(WorkspaceListRow {
        name: ws.name.clone(),
        is_root: ws.is_root,
        git_status,
        docker: Some(DockerCells {
            status: wsdc.status(),
            created: wsdc.created(),
            dc_managed: wsdc.dc_managed(),
            stats,
            execs,
            fwd_ports,
            docker_ports: wsdc.docker_ports(),
        }),
    })
}
