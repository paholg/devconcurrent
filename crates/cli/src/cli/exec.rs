use std::io::IsTerminal;
use std::os::unix::process::CommandExt;

use clap::Args;
use clap_complete::ArgValueCompleter;
use docker::ContainerStatus;
use eyre::eyre;
use indexmap::IndexMap;

use crate::cli::State;
use crate::complete::complete_workspace;
use crate::devcontainer::substitution;
use crate::docker::probe;
use crate::state::DevcontainerState;

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
        let workspace_full = workspace.devcontainer(devcontainer).await?;
        if workspace_full.status() != Some(ContainerStatus::Running) {
            return Err(eyre!(
                "workspace is not running: {}",
                workspace.path.display()
            ));
        }
        let container_id = workspace_full.service_container_id()?;
        let container =
            probe::ContainerData::inspect(&devcontainer.docker.client, container_id).await?;
        let probed = probe::user_env(
            container_id,
            devcontainer.config.remote_user.as_deref(),
            &container.env,
            devcontainer.config.user_env_probe,
        )
        .await?;
        let context =
            substitution::Context::new(&workspace.path, &devcontainer.config.workspace_folder)
                .with_container(container);
        let mut remote_env: IndexMap<String, Option<String>> =
            probed.into_iter().map(|(k, v)| (k, Some(v))).collect();
        for (key, template) in &devcontainer.config.remote_env {
            remote_env.insert(key.clone(), template.as_ref().map(|t| t.render(&context)));
        }

        exec_interactive(container_id, devcontainer, &remote_env, &self.cmd)
    }
}

pub(crate) fn exec_interactive(
    container_id: &str,
    devcontainer: &DevcontainerState,
    remote_env: &IndexMap<String, Option<String>>,
    cmd_args: &[String],
) -> eyre::Result<()> {
    let mut cmd = std::process::Command::new("docker");
    cmd.arg("exec");
    if std::io::stdin().is_terminal() {
        cmd.arg("-it");
    }

    let dc_options = devcontainer.devconcurrent();

    if let Some(u) = devcontainer.config.remote_user.as_deref() {
        cmd.args(["-u", u]);
    }
    cmd.arg("-w").arg(&devcontainer.config.workspace_folder);

    for (k, v) in remote_env {
        // null in remoteEnv means "unset" per spec; we can't truly unset PID-1-inherited vars via
        // `docker exec`, so set to empty string — closer to intent than the reference's literal
        // "null" stringification.
        cmd.arg("-e")
            .arg(format!("{k}={}", v.as_deref().unwrap_or("")));
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
