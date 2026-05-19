use std::path::Path;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::State;
use crate::complete::complete_workspace;
use crate::config::Config;
use crate::helpers::forward_to_shell;

/// Cd into the workspace directory (only if using via shell wrapper).
#[derive(Debug, Args)]
pub(crate) struct Go {
    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: String,
}

impl Go {
    pub(crate) async fn run(self, project: Option<String>) -> eyre::Result<()> {
        let config = Config::load()?;
        let state = State::new(project, &config).await?;
        let ws = state.resolve_workspace(Some(self.workspace)).await?;
        go(&ws.path)
    }
}

pub(crate) fn go(path: &Path) -> eyre::Result<()> {
    let path_str = path.to_string_lossy();
    let quoted = shlex::try_quote(&path_str)?;
    forward_to_shell(&format!("cd {quoted}"))
}
