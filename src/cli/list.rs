use clap::Args;
use owo_colors::OwoColorize;

use crate::{
    archive,
    cli::State,
    workspace::{Workspace, table::workspace_table},
};

/// List all workspaces for the project
#[derive(Debug, Args)]
pub struct List;

impl List {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let workspaces = Workspace::list(&state).await?;
        eprint!("{}", workspace_table(&workspaces));

        let archived = archive::list_archived(&state.project_name);
        if !archived.is_empty() {
            eprint!("\n{}", "Archived".dimmed());
            for dw in &archived {
                eprint!(" {}", dw.workspace_name);
            }
            eprintln!();
        }

        Ok(())
    }
}
