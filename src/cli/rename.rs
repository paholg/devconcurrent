use std::path::Path;

use clap::Args;
use clap_complete::ArgValueCompleter;
use eyre::{Context, eyre};
use tokio::process::Command;

use crate::cli::State;
use crate::complete::complete_workspace;
use crate::docker::compose::{compose_args, compose_project_name, compose_up_args};
use crate::run::cmd::{Cmd, NamedCmd};
use crate::run::{self, Runner};

/// Rename a workspace, preserving volume data
#[derive(Debug, Args)]
pub struct Rename {
    /// Current workspace name
    #[arg(add = ArgValueCompleter::new(complete_workspace))]
    from: String,

    /// New workspace name
    to: String,
}

impl Rename {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let from = &self.from;
        let to = &self.to;

        if state.is_root(from) {
            return Err(eyre!("cannot rename root workspace"));
        }

        if Path::new(to)
            .file_name()
            .is_none_or(|f| f != std::ffi::OsStr::new(to))
        {
            eyre::bail!("invalid workspace name: {to:?}");
        }

        let devcontainer = state.devcontainer()?;
        let dc_options = &devcontainer.common.customizations.devconcurrent;
        let workspace_dir = dc_options.workspace_dir(&state.project.path);

        let old_path = workspace_dir.join(from);
        if !old_path.exists() {
            return Err(eyre!("no workspace named '{from}' found"));
        }

        let new_path = workspace_dir.join(to);
        if new_path.exists() {
            return Err(eyre!("workspace '{to}' already exists"));
        }

        let old_project = compose_project_name(&old_path);

        // Down old workspace (keep volumes)
        eprintln!("Stopping workspace '{from}'...");
        docker(&["compose", "-p", &old_project, "down", "--remove-orphans"]).await?;

        // Remove fwd sidecars for this workspace
        remove_fwd_sidecars(&old_project).await?;

        let vol_count = rename_workspace(&state, &old_path, &new_path).await?;

        // Bring up new workspace
        let crate::devcontainer::Kind::Compose(ref compose) = devcontainer.kind else {
            unimplemented!();
        };

        let base_args = compose_args(
            &devcontainer,
            compose,
            &new_path,
            &state.project_name,
            &state.project,
        )?;
        let up_args = compose_up_args(compose, &base_args);
        let cmd = NamedCmd {
            name: "docker compose up",
            cmd: &Cmd::Args(up_args),
            dir: None,
        };
        Runner::run(cmd).await?;

        eprintln!("Renamed workspace '{from}' to '{to}'");
        if vol_count > 0 {
            eprintln!("Note: {vol_count} backing volume(s) retained from the old workspace name",);
        }

        Ok(())
    }
}

/// Core rename logic: create bind-mount alias volumes, move worktree, clean up.
/// Returns the number of volumes aliased.
pub async fn rename_workspace(
    state: &State,
    old_path: &Path,
    new_path: &Path,
) -> eyre::Result<usize> {
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

    // Clean up old compose override file
    let old_override = std::env::temp_dir().join(format!("{old_project}-override.yml"));
    if old_override.exists() {
        std::fs::remove_file(&old_override)
            .wrap_err_with(|| format!("failed to remove {}", old_override.display()))?;
    }

    Ok(vol_count)
}

async fn volume_mountpoint(vol_name: &str) -> eyre::Result<String> {
    let out = Command::new("docker")
        .args([
            "volume",
            "inspect",
            "--format",
            "{{ .Mountpoint }}",
            vol_name,
        ])
        .output()
        .await?;
    eyre::ensure!(out.status.success(), "docker volume inspect failed");
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

/// If this volume is a bind-mount alias (from a prior rename), return the
/// original backing volume name. Returns `None` for normal volumes.
pub(crate) async fn resolve_backing_volume(vol_name: &str) -> Option<String> {
    let out = Command::new("docker")
        .args([
            "volume",
            "inspect",
            "--format",
            "{{ index .Labels \"dev.dc.backing_volume\" }}",
            vol_name,
        ])
        .output()
        .await
        .ok()?;
    let label = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if label.is_empty() { None } else { Some(label) }
}

/// List volumes belonging to a compose project.
pub(crate) async fn list_project_volumes(compose_project: &str) -> eyre::Result<Vec<String>> {
    let filter = format!("label=com.docker.compose.project={compose_project}");
    let out = Command::new("docker")
        .args(["volume", "ls", "-q", "--filter", &filter])
        .output()
        .await?;
    eyre::ensure!(out.status.success(), "docker volume ls failed");
    let stdout = String::from_utf8(out.stdout)?;
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Remove port-forward sidecar containers and volumes for a workspace.
pub(crate) async fn remove_fwd_sidecars(compose_project: &str) -> eyre::Result<()> {
    let filter = format!("label=dev.dc.workspace={compose_project}");

    // Remove containers
    let out = Command::new("docker")
        .args(["ps", "-a", "-q", "--filter", &filter])
        .output()
        .await?;
    let stdout = String::from_utf8(out.stdout)?;
    let ids: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    if !ids.is_empty() {
        let mut args = vec!["rm", "-f"];
        args.extend(ids);
        docker(&args).await?;
    }

    // Remove volumes
    let out = Command::new("docker")
        .args(["volume", "ls", "-q", "--filter", &filter])
        .output()
        .await?;
    let stdout = String::from_utf8(out.stdout)?;
    for vol in stdout.lines().filter(|l| !l.is_empty()) {
        let _ = docker(&["volume", "rm", vol]).await;
    }

    Ok(())
}

pub(crate) async fn docker(args: &[&str]) -> eyre::Result<()> {
    let out = Command::new("docker").args(args).output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(eyre!("docker {} failed: {}", args[0], stderr.trim()));
    }
    Ok(())
}
