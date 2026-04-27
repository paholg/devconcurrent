use std::borrow::Cow;
use std::collections::HashMap;

use bollard::query_parameters::{ListContainersOptions, RemoveContainerOptions};
use clap::Args;
use clap_complete::ArgValueCompleter;
use eyre::eyre;

use crate::ansi::{RED, RESET, YELLOW};
use crate::cli::{State, confirm, safety_check};
use crate::complete::complete_workspace;
use crate::docker::compose::{compose_cmd, remove_override_file};
use crate::run::{self, Runnable, Runner, run_command};
use crate::state::DevcontainerState;
use crate::workspace::{Workspace, WorkspaceMini};

/// Fully destroy the workspace; equivalent to `docker compose down -v --remove-orphans && git worktree remove`
#[derive(Debug, Args)]
pub(crate) struct Destroy {
    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: Option<String>,

    /// Force remove the worktree, even if dirty
    #[arg(short, long)]
    force: bool,
}

impl Destroy {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        let devcontainer = state.try_devcontainer()?;
        let workspace = state.resolve_workspace(self.workspace).await?;
        let workspace_full = Workspace::get(&state, devcontainer, &workspace.name).await?;

        if !workspace.path.exists() {
            return Err(eyre!("workspace '{}' not found", workspace.name));
        }

        safety_check(&workspace_full, self.force)?;

        if workspace.root {
            eprintln!(
                "{YELLOW}Will destroy {RED}root{YELLOW} workspace — DATA WILL BE LOST{RESET}",
            );
            if !confirm()? {
                eprintln!("Aborted.");
                return Ok(());
            }
        }

        let cleanup = Cleanup {
            state: &state,
            devcontainer,
            workspace: &workspace,
            force: self.force,
        };

        Runner::run(cleanup).await
    }
}

struct Cleanup<'a> {
    state: &'a State,
    devcontainer: &'a DevcontainerState,
    workspace: &'a WorkspaceMini,
    force: bool,
}

impl Runnable for Cleanup<'_> {
    fn name(&self) -> Cow<'_, str> {
        (&self.workspace.name).into()
    }

    fn description(&self) -> Cow<'_, str> {
        format!("destroy {}", self.workspace.path.display()).into()
    }

    async fn run(self, _: run::Token) -> eyre::Result<()> {
        let mut down_cmd = compose_cmd(self.state, self.devcontainer, self.workspace)?;
        down_cmd.args(["down", "-v", "--remove-orphans"]);

        run_command(down_cmd).await?;
        remove_override_file(self.state, self.workspace);

        // Remove any port-forward sidecars targeting this workspace
        let mut filters = HashMap::new();
        filters.insert("label".into(), self.workspace.docker_labels(self.state));

        let docker = &self.devcontainer.docker.docker;

        if let Ok(containers) = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await
        {
            for c in containers {
                if let Some(id) = c.id {
                    let _ = docker
                        .remove_container(
                            &id,
                            Some(RemoveContainerOptions {
                                force: true,
                                ..Default::default()
                            }),
                        )
                        .await;
                }
            }
        }

        if !self.workspace.root {
            let mut worktree_cmd = tokio::process::Command::new("git");
            worktree_cmd.args(["worktree", "remove"]);

            if self.force {
                worktree_cmd.arg("--force");
            }
            worktree_cmd.arg(&self.workspace.path);
            worktree_cmd.current_dir(&self.state.project.path);

            run_command(worktree_cmd).await?;
        }

        eprintln!("Removed {}", self.workspace.path.display());
        Ok(())
    }
}
