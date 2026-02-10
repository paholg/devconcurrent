use crate::ansi::{RED, RESET, YELLOW};
use crate::cli::State;
use crate::run::Runner;
use crate::workspace::Workspace;
use clap::Args;
use eyre::eyre;

use super::prune::{Cleanup, confirm};

/// Destroy a workspace by name, removing its containers and worktree.
///
/// Unlike `prune`, this does not skip dirty or in-use workspaces.
#[derive(Debug, Args)]
pub struct Kill {
    #[arg(help = "name of the workspace to destroy")]
    name: String,

    #[arg(short, long, help = "force remove worktrees")]
    force: bool,
}

impl Kill {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let workspace = Workspace::get(&state, Some(&self.name)).await?;

        let is_root = workspace.path == state.project.path;

        if !workspace.path.exists() {
            return Err(eyre!("no workspace named '{}' found", self.name));
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

        let cleanup = Cleanup {
            docker: &state.docker.docker,
            repo_path: &state.project.path,
            path: &workspace.path,
            compose_name: super::up::compose_project_name(&workspace.path),
            remove_worktree: !is_root,
            force: self.force,
        };

        Runner::run(cleanup).await
    }
}
