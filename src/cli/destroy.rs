use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;

use bollard::Docker;
use bollard::query_parameters::{ListContainersOptions, RemoveContainerOptions};
use clap::Args;
use clap_complete::ArgValueCompleter;
use eyre::{Context, eyre};

use crate::ansi::{RED, RESET, YELLOW};
use crate::archive;
use crate::cli::State;
use crate::cli::rename::resolve_backing_volume;
use crate::complete::complete_workspace;
use crate::docker::compose::compose_project_name;
use crate::run::{self, Runnable, Runner, run_cmd};
use crate::workspace::Workspace;

/// Fully destroy the workspace; equivalent to `docker compose down -v --remove-orphans && git worktree remove`
#[derive(Debug, Args)]
pub struct Destroy {
    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: String,

    /// Force remove the worktree, even if dirty
    #[arg(short, long)]
    force: bool,
}

impl Destroy {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let name = self.workspace;
        let workspace = Workspace::get(&state, &name).await?;

        let is_root = workspace.path == state.project.path;

        if !workspace.path.exists() {
            return Err(eyre!("no workspace named '{}' found", name));
        }

        if is_root {
            eprintln!(
                "{YELLOW}Will destroy {RED}root{YELLOW} workspace — DATA WILL BE LOST{RESET}",
            );
            if !confirm()? {
                eprintln!("Aborted.");
                return Ok(());
            }
        }

        let compose_name = compose_project_name(&workspace.path);

        // Remove archived marker if one exists
        if let Err(e) = archive::unarchive(&state.project_name, &compose_name) {
            eprintln!("warning: failed to remove archive marker: {e}");
        }

        let cleanup = Cleanup {
            docker: &state.docker.docker,
            repo_path: &state.project.path,
            path: &workspace.path,
            compose_name,
            remove_worktree: !is_root,
            force: self.force,
        };

        Runner::run(cleanup).await
    }
}

struct Cleanup<'a> {
    docker: &'a Docker,
    repo_path: &'a Path,
    path: &'a Path,
    compose_name: String,
    remove_worktree: bool,
    force: bool,
}

impl Runnable for Cleanup<'_> {
    fn name(&self) -> Cow<'_, str> {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or(self.path.display().to_string().into())
    }

    fn description(&self) -> Cow<'_, str> {
        format!("destroy {}", self.path.display()).into()
    }

    async fn run(self, _: run::Token) -> eyre::Result<()> {
        // Collect backing volumes before tearing down (these are from renamed workspaces)
        let backing_volumes = backing_volumes(&self.compose_name).await;

        run_cmd(
            &[
                "docker",
                "compose",
                "-p",
                &self.compose_name,
                "down",
                "-v",
                "--remove-orphans",
            ],
            None,
        )
        .await?;

        // Remove backing volumes from a prior rename
        for vol in backing_volumes {
            let _ = tokio::process::Command::new("docker")
                .args(["volume", "rm", &vol])
                .output()
                .await;
        }

        let override_file =
            std::env::temp_dir().join(format!("{}-override.yml", self.compose_name));
        if override_file.exists() {
            std::fs::remove_file(&override_file)
                .wrap_err_with(|| format!("failed to remove {}", override_file.display()))?;
        }

        // Remove any port-forward sidecar targeting this workspace
        let mut filters = HashMap::new();
        filters.insert(
            "label".into(),
            vec![format!("dev.dc.workspace={}", self.compose_name)],
        );
        if let Ok(containers) = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await
        {
            for c in containers {
                if let Some(id) = c.id {
                    let _ = self
                        .docker
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

        if self.remove_worktree {
            let mut args = vec!["git", "worktree", "remove"];
            if self.force {
                args.push("--force");
            }
            let path_str = self.path.to_string_lossy();
            args.push(&path_str);

            run_cmd(&args, Some(self.repo_path)).await?;
        }

        eprintln!("Removed {}", self.path.display());
        Ok(())
    }
}

/// Find backing volumes from a prior rename. These are old volumes whose data
/// is bind-mounted into the current workspace's volumes.
async fn backing_volumes(compose_name: &str) -> Vec<String> {
    let Ok(volumes) = crate::cli::rename::list_project_volumes(compose_name).await else {
        return Vec::new();
    };
    let mut backing = Vec::new();
    for vol in &volumes {
        if let Some(bv) = resolve_backing_volume(vol).await {
            backing.push(bv);
        }
    }
    backing
}

pub(super) fn confirm() -> eyre::Result<bool> {
    eprint!("Proceed? [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}
