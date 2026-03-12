use clap::Args;
use clap_complete::ArgValueCompleter;
use color_eyre::owo_colors::OwoColorize;
use tracing::info_span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

use std::path::Path;

use eyre::Context;

use crate::archive;
use crate::cli::State;
use crate::cli::copy::copy_volumes;
use crate::cli::exec::exec_interactive;
use crate::cli::fwd::forward;
use crate::complete::complete_workspace;
use crate::docker::compose::{
    compose_args, compose_project_name, compose_ps_q, compose_up_args, docker,
    list_project_volumes, remove_fwd_sidecars, remove_override_file, resolve_backing_volume,
    volume_mountpoint,
};
use crate::run::cmd::{Cmd, NamedCmd};
use crate::run::{self, Runner};
use crate::worktree;

/// Bring up a workspace, creating it if it does not exist
#[derive(Debug, Args)]
pub struct Work {
    /// Copy configured `defaultCopyVolumes` from root workspace
    #[arg(short, long)]
    copy: bool,

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

impl Work {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let devcontainer = state.devcontainer()?;
        let dc_options = &devcontainer.common.customizations.devconcurrent;

        let name = state.resolve_workspace(self.workspace).await?;
        let is_root = state.is_root(&name);
        let workspace_dir = dc_options.workspace_dir(&state.project.path);
        let worktree_path = if is_root {
            state.project.path.clone()
        } else if !self.copy {
            if let Some(reused) =
                try_reuse_archived(&state, &workspace_dir, &name, self.detach).await?
            {
                reused
            } else {
                worktree::create(&state.project.path, &workspace_dir, &name, self.detach).await?
            }
        } else {
            worktree::create(&state.project.path, &workspace_dir, &name, self.detach).await?
        };

        // Set up span.
        let name = &name;
        let colored_name = name.cyan().to_string();
        let up = "work".cyan().to_string();
        let path = worktree_path.display().to_string();
        let description = &path;
        let message = format!(
            "Spinning up workspace {colored_name} from root {}",
            state.project.path.display()
        );
        let pb_message = format!("[{up}] Spinning up workspace {colored_name}");
        let finish_message = format!("Workspace {colored_name} is available.");
        let span = info_span!(
            "work",
            indicatif.pb_show = true,
            name = up,
            description,
            message,
            finish_message
        );
        span.pb_set_message(&pb_message);
        let _guard = span.enter();

        let crate::devcontainer::Kind::Compose(ref compose) = devcontainer.kind else {
            // This was handled at deserialize time already.
            unimplemented!();
        };

        let base_args = compose_args(
            &devcontainer,
            compose,
            &worktree_path,
            &state.project_name,
            &state.project,
        )?;

        // initializeCommand runs on the host, from the worktree
        if let Some(ref cmd) = devcontainer.common.initialize_command {
            cmd.run_on_host("initializeCommand", Some(&worktree_path))
                .await?;
        }

        if self.copy && !is_root {
            let root_project = compose_project_name(&state.project.path);
            let new_project = compose_project_name(&worktree_path);

            copy_volumes(&state, Vec::new(), &root_project, &new_project).await?;
        }

        let up_args = compose_up_args(compose, &base_args);
        let cmd = NamedCmd {
            name: "docker compose up",
            cmd: &Cmd::Args(up_args),
            dir: None,
        };
        Runner::run(cmd).await?;

        let container_id = compose_ps_q(compose, &base_args).await?;
        let user = devcontainer.common.remote_user.as_deref();
        let workdir = Some(compose.workspace_folder.as_path());
        let remote_env = &devcontainer.common.remote_env;

        // Lifecycle commands: create-only commands run only on first creation
        // For now, though, we always recreate.
        if let Some(ref cmd) = devcontainer.common.on_create_command {
            cmd.run_in_container("onCreateCommand", &container_id, user, workdir, remote_env)
                .await?;
        }
        if let Some(ref cmd) = devcontainer.common.update_content_command {
            cmd.run_in_container(
                "updateContentCommand",
                &container_id,
                user,
                workdir,
                remote_env,
            )
            .await?;
        }
        if let Some(ref cmd) = devcontainer.common.post_create_command {
            cmd.run_in_container(
                "postCreateCommand",
                &container_id,
                user,
                workdir,
                remote_env,
            )
            .await?;
        }
        if let Some(ref cmd) = devcontainer.common.post_start_command {
            cmd.run_in_container("postStartCommand", &container_id, user, workdir, remote_env)
                .await?;
        }

        // Port forward if requested
        if self.forward {
            forward(&state, name).await?;
        }

        // Interactive exec if requested
        if let Some(cmd_args) = self.exec {
            exec_interactive(
                &container_id,
                user,
                workdir,
                &cmd_args,
                dc_options.default_exec.as_ref(),
                &state.project.exec.environment,
            )?;
        }

        Ok(())
    }
}

/// Try to reuse an archived workspace's volumes and worktree via rename.
/// Returns the new worktree path if reuse succeeded, or None if no archived workspace available.
async fn try_reuse_archived(
    state: &State,
    workspace_dir: &std::path::Path,
    new_name: &str,
    detach: bool,
) -> eyre::Result<Option<std::path::PathBuf>> {
    let archived = match archive::find_archived(&state.project_name)? {
        Some(aw) => aw,
        None => return Ok(None),
    };

    let old_path = workspace_dir.join(&archived.workspace_name);
    let new_path = workspace_dir.join(new_name);

    if new_path.exists() {
        return Ok(None);
    }
    if !old_path.exists() {
        // Stale marker — clean it up
        let _ = archive::unarchive(&state.project_name, &archived.compose_project);
        return Ok(None);
    }

    eprintln!(
        "Reusing archived workspace '{}' as '{new_name}'...",
        archived.workspace_name
    );

    // Safe to re-run down even though `archive` already stopped it
    docker(&[
        "compose",
        "-p",
        &archived.compose_project,
        "down",
        "--remove-orphans",
    ])
    .await?;
    remove_fwd_sidecars(&archived.compose_project).await?;

    rename_workspace(state, &old_path, &new_path).await?;

    // Set up the branch, matching what `git worktree add` would do.
    let root_head = worktree::rev_parse_head(&state.project.path).await?;
    if detach {
        run::run_cmd(
            &["git", "checkout", "--detach", &root_head],
            Some(&new_path),
        )
        .await?;
    } else {
        run::run_cmd(
            &["git", "checkout", "-b", new_name, &root_head],
            Some(&new_path),
        )
        .await?;
    }

    archive::unarchive(&state.project_name, &archived.compose_project)?;

    Ok(Some(new_path))
}

/// Core rename logic: create bind-mount alias volumes, move worktree, clean up.
/// Returns the number of volumes aliased.
async fn rename_workspace(state: &State, old_path: &Path, new_path: &Path) -> eyre::Result<usize> {
    let old_project = compose_project_name(old_path);
    let new_project = compose_project_name(new_path);

    // Create bind-mount alias volumes.
    // If the old volumes are themselves aliases from a prior rename, follow
    // the chain back to the original backing volume so we never build up a
    // chain of bind-mount dependencies.
    let old_volumes = list_project_volumes(&old_project).await?;
    let mut intermediate_volumes: Vec<String> = Vec::new();
    for old_vol in &old_volumes {
        let suffix = old_vol
            .strip_prefix(&format!("{old_project}_"))
            .unwrap_or(old_vol);
        let new_vol = format!("{new_project}_{suffix}");

        let backing = resolve_backing_volume(old_vol).await;
        let is_alias = backing.is_some();
        let backing = backing.as_deref().unwrap_or(old_vol);

        let mountpoint = volume_mountpoint(backing)
            .await
            .wrap_err_with(|| format!("failed to get mountpoint for volume {backing}"))?;
        let device = format!("device={mountpoint}");
        docker(&[
            "volume",
            "create",
            "--driver",
            "local",
            "--opt",
            "type=none",
            "--opt",
            "o=bind",
            "--opt",
            &device,
            "--label",
            &format!("com.docker.compose.project={new_project}"),
            "--label",
            &format!("com.docker.compose.volume={suffix}"),
            "--label",
            &format!("dev.dc.backing_volume={backing}"),
            &new_vol,
        ])
        .await
        .wrap_err_with(|| format!("failed to create alias volume {new_vol}"))?;

        // The old volume was itself an alias — it can be removed now.
        if is_alias {
            intermediate_volumes.push(old_vol.clone());
        }
    }

    // Clean up intermediate alias volumes from the prior rename.
    for vol in &intermediate_volumes {
        let _ = docker(&["volume", "rm", vol]).await;
    }

    let vol_count = old_volumes.len();

    // Move the worktree
    let old_path_str = old_path.to_string_lossy();
    let new_path_str = new_path.to_string_lossy();
    run::run_cmd(
        &["git", "worktree", "move", &old_path_str, &new_path_str],
        Some(&state.project.path),
    )
    .await
    .wrap_err("failed to move worktree")?;

    remove_override_file(&old_project);

    Ok(vol_count)
}
