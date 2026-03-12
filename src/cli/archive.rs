use clap::Args;
use clap_complete::ArgValueCompleter;
use eyre::eyre;

use crate::archive;
use crate::cli::State;
use crate::cli::rename::{docker, remove_fwd_sidecars};
use crate::complete::complete_workspace;
use crate::docker::compose::compose_project_name;

/// Archive a workspace, stopping containers but preserving volumes for reuse
#[derive(Debug, Args)]
pub struct Archive {
    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: String,
}

impl Archive {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let name = &self.workspace;

        if state.is_root(name) {
            return Err(eyre!("cannot archive root workspace"));
        }

        let devcontainer = state.devcontainer()?;
        let dc_options = &devcontainer.common.customizations.devconcurrent;
        let workspace_dir = dc_options.workspace_dir(&state.project.path);

        let ws_path = workspace_dir.join(name);
        if !ws_path.exists() {
            return Err(eyre!("no workspace named '{name}' found"));
        }

        let compose_project = compose_project_name(&ws_path);

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

        archive::archive(&state.project_name, &compose_project, name)?;

        eprintln!("Workspace '{name}' archived (volumes preserved for reuse)");
        Ok(())
    }
}
