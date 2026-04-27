use clap::Args;

use crate::{
    cli::State,
    workspace::{WorkspaceLegacy, table::workspace_table},
};

/// List all workspaces for the project
#[derive(Debug, Args)]
pub(crate) struct List;

impl List {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        // TODO: This command should not require devcontainer state.
        let devcontainer = state.try_devcontainer()?;
        let workspaces = WorkspaceLegacy::list(&state, devcontainer).await?;
        eprint!("{}", workspace_table(&workspaces));
        Ok(())
    }
}
