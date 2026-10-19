use std::path::Path;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::{State, either_workspace};
use crate::complete::complete_workspace;
use crate::config::Config;
use crate::helpers::forward_to_shell;

/// Cd into the workspace directory (only if using via shell wrapper).
#[derive(Debug, Args)]
pub(crate) struct Go {
    /// Workspace name
    #[arg(value_name = "WORKSPACE", add = ArgValueCompleter::new(complete_workspace))]
    workspace_name: Option<String>,
}

impl Go {
    pub(crate) async fn run(
        self,
        project: Option<String>,
        workspace: Option<String>,
    ) -> eyre::Result<()> {
        let config = Config::load()?;
        let state = State::new(project, &config).await?;
        let name = either_workspace(self.workspace_name, workspace)?
            .ok_or_else(|| eyre::eyre!("no workspace given"))?;
        let ws = state.resolve_workspace(Some(name)).await?;
        go(&ws.path)
    }
}

pub(crate) fn go(path: &Path) -> eyre::Result<()> {
    let path_str = path.to_string_lossy();
    let quoted = shlex::try_quote(&path_str)?;
    forward_to_shell(&format!("cd {quoted}"))
}
