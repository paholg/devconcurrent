use clap::Args;
use clap_complete::ArgValueCompleter;
use eyre::eyre;

use crate::archive;
use crate::cli::{State, safety_check};
use crate::complete::complete_workspace;
use crate::docker::compose::{
    compose_project_name, docker, remove_fwd_sidecars, remove_override_file,
};
use crate::run::run_cmd;
use crate::workspace::Workspace;

/// Archive a workspace, stopping containers but preserving volumes for reuse
#[derive(Debug, Args)]
pub struct Archive {
    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: String,

    /// Force archive even if dirty or has active execs
    #[arg(short, long)]
    force: bool,
}

impl Archive {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let name = &self.workspace;

        if state.is_root(name) {
            return Err(eyre!("cannot archive root workspace"));
        }

        let workspace = Workspace::get(&state, name).await?;

        if !workspace.path.exists() {
            return Err(eyre!("no workspace named '{name}' found"));
        }

        let compose_project = compose_project_name(&workspace.path);

        if archive::is_archived(&state.project_name, &compose_project) {
            eprintln!("Workspace '{name}' is already archived.");
            return Ok(());
        }

        safety_check(&workspace, self.force)?;

        run_cmd(&["git", "checkout", "--detach"], Some(&workspace.path)).await?;

        eprintln!("Stopping workspace '{name}'...");
        docker(&[
            "compose",
            "-p",
            &compose_project,
            "down",
            "--remove-orphans",
        ])
        .await?;

        remove_fwd_sidecars(&compose_project).await?;
        remove_override_file(&compose_project);

        archive::archive(&state.project_name, &compose_project, name)?;

        eprintln!("Workspace '{name}' archived (volumes preserved for reuse)");
        Ok(())
    }
}
