use std::path::PathBuf;

use eyre::{Context, eyre};
use serde_json::json;

use crate::{
    state::{DevcontainerState, State},
    workspace::WorkspaceMini,
};

fn override_path(state: &State, workspace: &WorkspaceMini) -> PathBuf {
    state
        .project_working_dir()
        .join(format!("{}-override.yml", workspace.name))
}

pub(crate) fn remove_override_file(state: &State, workspace: &WorkspaceMini) {
    let path = override_path(state, workspace);

    if path.exists()
        && let Err(e) = std::fs::remove_file(&path)
    {
        eprintln!("warning: failed to remove {}: {e}", path.display());
    }
}

/// Write the compose override and return docker compose base args.
pub(crate) fn compose_cmd(
    state: &State,
    devcontainer: &DevcontainerState,
    workspace: &WorkspaceMini,
) -> eyre::Result<tokio::process::Command> {
    let override_file_path = write_compose_override(state, devcontainer, workspace)?;

    let mut cmd = tokio::process::Command::new("docker");

    cmd.args(["compose", "-p"])
        .arg(workspace.compose_project_name());

    for f in &devcontainer.compose().docker_compose_file {
        cmd.arg("-f")
            .arg(workspace.path.join(".devcontainer").join(f));
    }

    cmd.arg("-f").arg(override_file_path);
    Ok(cmd)
}

pub(crate) async fn compose_ps_q(
    state: &State,
    devcontainer: &DevcontainerState,
    workspace: &WorkspaceMini,
) -> eyre::Result<String> {
    let mut cmd = compose_cmd(state, devcontainer, workspace)?;

    let service = &devcontainer.compose().service;
    cmd.arg("ps").arg("-q").arg(service);

    let out = cmd.output().await?;
    eyre::ensure!(out.status.success(), "docker compose ps failed");
    let output = String::from_utf8(out.stdout)?;
    let id = output.lines().next().unwrap_or("").trim().to_string();
    if id.is_empty() {
        return Err(eyre!("no container found for service '{}'", service));
    }
    Ok(id)
}

/// Generate a compose override file
///
/// We set the standard devcontainer labels, our own labels, and any appropriate overrides from
/// devcontainer.json.
fn write_compose_override(
    state: &State,
    devcontainer: &DevcontainerState,
    workspace: &WorkspaceMini,
) -> eyre::Result<PathBuf> {
    let override_path = override_path(state, workspace);

    let mut service_obj = json!({
        "labels": [
            format!("devcontainer.local_folder={}", workspace.path.display()),
            format!("devcontainer.config_file={}", devcontainer.path.display()),
            "dev.devconcurrent.managed=true".to_string(),
            format!("dev.devconcurrent.project={}", state.project_name),
        ]
    });

    let common = &devcontainer.config.common;
    let mut env = state.project.environment.clone();
    env.extend(
        common
            .container_env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    if !env.is_empty() {
        service_obj["environment"] = json!(env);
    }

    if let Some(init) = common.init {
        service_obj["init"] = json!(init);
    }
    if let Some(privileged) = common.privileged {
        service_obj["privileged"] = json!(privileged);
    }
    if !common.cap_add.is_empty() {
        service_obj["cap_add"] = json!(common.cap_add);
    }
    if !common.security_opt.is_empty() {
        service_obj["security_opt"] = json!(common.security_opt);
    }
    if let Some(ref user) = common.container_user {
        service_obj["user"] = json!(user);
    }

    let devconcurrent_options = devcontainer.devconcurrent();

    let mut volumes = state.project.volumes.clone();
    if devconcurrent_options.mount_git && !workspace.root {
        let git_dir = state.project.path.join(".git");
        volumes.push(format!("{}:{}", git_dir.display(), git_dir.display()));
    }
    if !volumes.is_empty() {
        service_obj["volumes"] = json!(volumes);
    }

    if devcontainer.compose().override_command {
        // I believe this is the reference devcontainer overrideCommand.
        service_obj["entrypoint"] = json!([
            "/bin/sh",
            "-c",
            r#"echo Container started
 trap "exit 0" 15

 exec "$@"
 while sleep 1 & wait $!; do :; done"#,
            "-"
        ]);
        service_obj["command"] = json!([]);
    }

    let content = serde_json::to_string_pretty(&json!({
        "services": { &devcontainer.compose().service: service_obj }
    }))?;

    state.ensure_project_working_dir()?;
    std::fs::write(&override_path, content)
        .wrap_err_with(|| format!("failed to write {}", override_path.display()))?;
    Ok(override_path)
}
