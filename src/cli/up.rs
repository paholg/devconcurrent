use clap::Args;
use clap_complete::ArgValueCompleter;
use color_eyre::owo_colors::OwoColorize;
use tracing::info_span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

use crate::cli::State;
use crate::cli::exec::exec_interactive;
use crate::cli::fwd::forward;
use crate::complete::complete_workspace;
use crate::docker::compose::{compose_cmd, compose_ps_q};
use crate::run::Runner;
use crate::run::cmd::NamedCmd;
use crate::worktree;

/// Bring up a workspace, creating it if it does not exist
#[derive(Debug, Args)]
pub(crate) struct Up {
    /// Foward configured `forwardPorts` once up
    #[arg(short, long)]
    forward: bool,

    /// Detach worktree rather than creating a branch
    #[arg(short, long)]
    detach: bool,

    /// Workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    workspace: Option<String>,

    /// Exec once up with the given command [default: configured defaultExec]
    #[arg(short = 'x', long, num_args = 0.., allow_hyphen_values = true)]
    exec: Option<Vec<String>>,
}

impl Up {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        let workspace = state.resolve_workspace(self.workspace).await?;
        let devcontainer = state.try_devcontainer()?;

        // Set up span.
        let name = &workspace.name;
        let colored_name = name.cyan().to_string();
        let up = "up".cyan().to_string();
        let path = workspace.path.display().to_string();
        if !workspace.root {
            worktree::create(&state.project.path, &workspace, self.detach).await?;
        }

        let description = &path;
        let message = format!(
            "Spinning up workspace {colored_name} from root {}",
            state.project.path.display()
        );
        let pb_message = format!("[{up}] Spinning up workspace {colored_name}");
        let finish_message = format!("Workspace {colored_name} is available.");
        let span = info_span!(
            "up",
            indicatif.pb_show = true,
            name = up,
            description,
            message,
            finish_message
        );
        span.pb_set_message(&pb_message);
        let _guard = span.enter();

        // initializeCommand runs on the host, from the worktree
        if let Some(ref cmd) = devcontainer.config.common.initialize_command {
            cmd.run_on_host("initializeCommand", Some(&workspace.path))
                .await?;
        }

        let mut compose_up_cmd = compose_cmd(&state, devcontainer, &workspace)?;
        compose_up_cmd.args(["up", "-d", "--build"]);

        let compose_config = devcontainer.compose();
        if let Some(ref services) = compose_config.run_services {
            compose_up_cmd.args(services);
            if !services.contains(&compose_config.service) {
                // TODO: We probably want this in the `else` also, or maybe we
                // don't need it at all?
                compose_up_cmd.arg(&compose_config.service);
            }
        }

        let up_cmd = compose_up_cmd.into_std().into();
        let cmd = NamedCmd {
            name: "docker compose up",
            cmd: &up_cmd,
            dir: None,
        };
        Runner::run(cmd).await?;

        let compose_config = devcontainer.compose();

        let container_id = compose_ps_q(&state, devcontainer, &workspace).await?;
        let user = devcontainer.config.common.remote_user.as_deref();
        let workdir = Some(compose_config.workspace_folder.as_path());
        let remote_env = &devcontainer.config.common.remote_env;

        // Lifecycle commands: create-only commands run only on first creation
        // For now, though, we always recreate.
        if let Some(ref cmd) = devcontainer.config.common.on_create_command {
            cmd.run_in_container("onCreateCommand", &container_id, user, workdir, remote_env)
                .await?;
        }
        if let Some(ref cmd) = devcontainer.config.common.update_content_command {
            cmd.run_in_container(
                "updateContentCommand",
                &container_id,
                user,
                workdir,
                remote_env,
            )
            .await?;
        }
        if let Some(ref cmd) = devcontainer.config.common.post_create_command {
            cmd.run_in_container(
                "postCreateCommand",
                &container_id,
                user,
                workdir,
                remote_env,
            )
            .await?;
        }
        if let Some(ref cmd) = devcontainer.config.common.post_start_command {
            cmd.run_in_container("postStartCommand", &container_id, user, workdir, remote_env)
                .await?;
        }

        // Port forward if requested
        if self.forward {
            forward(&state, devcontainer, &workspace).await?;
        }

        // Interactive exec if requested
        if let Some(cmd_args) = self.exec {
            exec_interactive(&container_id, &state, devcontainer, &cmd_args)?;
        }

        Ok(())
    }
}
