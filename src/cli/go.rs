use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::State;
use crate::complete::complete_workspace;
use crate::workspace::Workspace;

/// Move into the workspace directory (only if using via shell wrapper).
#[derive(Debug, Args)]
pub struct Go {
    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: String,
}

impl Go {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let ws = Workspace::get(&state, &self.workspace).await?;
        let path = ws.path.to_string_lossy();
        let quoted = shlex::try_quote(&path)?;
        println!("cd {quoted}");
        Ok(())
    }
}
