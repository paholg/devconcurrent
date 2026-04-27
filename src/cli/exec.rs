use std::io::IsTerminal;
use std::os::unix::process::CommandExt;

use bollard::plugin::ContainerSummaryStateEnum;
use clap::Args;
use clap_complete::ArgValueCompleter;
use eyre::eyre;

use crate::cli::State;
use crate::complete::complete_workspace;
use crate::state::DevcontainerState;
use crate::workspace::WorkspaceLegacy;

/// Exec into a running devcontainer
#[derive(Debug, Args)]
pub(crate) struct Exec {
    /// Workspace name [default: current working directory]
    #[arg(short, long, add = ArgValueCompleter::new(complete_workspace))]
    workspace: Option<String>,

    /// command to run [default: Configured defaultExec]
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    cmd: Vec<String>,
}

impl Exec {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        let workspace = state.resolve_workspace(self.workspace).await?;
        let devcontainer = state.try_devcontainer()?;
        let workspace_full = WorkspaceLegacy::get(&state, devcontainer, &workspace.name).await?;
        if workspace_full.status() != ContainerSummaryStateEnum::RUNNING {
            return Err(eyre!(
                "workspace is not running: {}",
                workspace.path.display()
            ));
        }
        let cid = workspace_full.service_container_id()?;

        exec_interactive(cid, &state, devcontainer, &self.cmd)
    }
}

pub(crate) fn exec_interactive(
    container_id: &str,
    state: &State,
    devcontainer: &DevcontainerState,
    cmd_args: &[String],
) -> eyre::Result<()> {
    let mut cmd = std::process::Command::new("docker");
    cmd.arg("exec");
    if std::io::stdin().is_terminal() {
        cmd.arg("-it");
    }

    let dc_options = devcontainer.devconcurrent();

    if let Some(u) = devcontainer.config.common.remote_user.as_deref() {
        cmd.args(["-u", u]);
    }
    cmd.arg("-w").arg(&devcontainer.compose().workspace_folder);

    for (k, v) in &state.project.exec.environment {
        let expanded = shellexpand::env(v)?;
        cmd.arg("-e").arg(format!("{k}={expanded}"));
    }
    cmd.arg(container_id);

    if cmd_args.is_empty() {
        cmd.args(
            dc_options
                .default_exec
                .as_ref()
                .ok_or_else(|| eyre!("no command provided and no default configured"))?
                .as_args(),
        );
    } else {
        cmd.args(cmd_args);
    }

    // Restore cursor visibility — indicatif hides it for spinners and exec()
    // replaces the process before indicatif's cleanup can run.
    let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Show);

    Err(cmd.exec().into())
}
