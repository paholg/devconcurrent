use std::collections::HashMap;

use bollard::plugin::ContainerSummaryStateEnum;
use clap::Args;
use futures::future::try_join_all;

use crate::{
    cli::State,
    docker::Stats,
    state::DevcontainerState,
    workspace::{
        Workspace,
        git_status::GitStatus,
        table::{WorkspaceListRow, workspace_table},
    },
};

/// List all workspaces for the project
#[derive(Debug, Args)]
pub(crate) struct List;

impl List {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        let workspaces = Workspace::list(&state).await?;
        let dc = state.devcontainer.as_ref();

        let fwd_ports_map = if let Some(dc) = dc {
            dc.docker.forwarded_ports(&state.project_name).await?
        } else {
            HashMap::new()
        };

        let rows = try_join_all(
            workspaces
                .iter()
                .map(|ws| build_row(ws, dc, &fwd_ports_map)),
        )
        .await?;

        eprint!("{}", workspace_table(&rows));
        Ok(())
    }
}

async fn build_row(
    ws: &Workspace<'_>,
    dc: Option<&DevcontainerState>,
    fwd_ports_map: &HashMap<String, Vec<u16>>,
) -> eyre::Result<WorkspaceListRow> {
    let git_future = GitStatus::fetch(&ws.path);

    let Some(dc) = dc else {
        let git_status = git_future.await?;
        return Ok(WorkspaceListRow {
            name: ws.name.clone(),
            is_root: ws.is_root,
            git_status,
            status: ContainerSummaryStateEnum::EMPTY,
            created: None,
            dc_managed: false,
            stats: Stats::default(),
            execs: 0,
            fwd_ports: Vec::new(),
            docker_ports: Vec::new(),
        });
    };

    let wsdc = ws.devcontainer(dc).await?;
    let (git_status, stats, execs) = tokio::try_join!(git_future, wsdc.stats(), wsdc.execs())?;

    let mut fwd_ports = fwd_ports_map.get(&ws.name).cloned().unwrap_or_default();
    fwd_ports.sort();
    fwd_ports.dedup();

    Ok(WorkspaceListRow {
        name: ws.name.clone(),
        is_root: ws.is_root,
        git_status,
        status: wsdc.status(),
        created: wsdc.created(),
        dc_managed: wsdc.dc_managed(),
        stats,
        execs,
        fwd_ports,
        docker_ports: wsdc.docker_ports(),
    })
}
