use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::State;
use crate::complete::complete_workspace;
use crate::helpers::forward_to_shell;

/// Cd into the workspace directory (only if using via shell wrapper).
#[derive(Debug, Args)]
pub(crate) struct Go {
    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: String,
}

impl Go {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        let ws = state.resolve_workspace(Some(self.workspace)).await?;
        let path = ws.path.to_string_lossy();
        let quoted = shlex::try_quote(&path)?;
        forward_to_shell(&format!("cd {quoted}"))
    }
}
