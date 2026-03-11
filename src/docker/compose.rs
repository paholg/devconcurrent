use std::path::{Path, PathBuf};

use eyre::{Context, eyre};
use serde_json::json;

use crate::{
    config::Project,
    devcontainer::{Compose, Devcontainer},
};

/// Match the devcontainer CLI convention: `{basename}_devcontainer`, lowercased,
/// keeping only `[a-z0-9-_]`.
pub fn compose_project_name(worktree_path: &Path) -> String {
    let basename = worktree_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let raw = format!("{basename}_devcontainer");
    raw.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn compose_base_args(
    compose: &Compose,
    worktree_path: &Path,
    override_file: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "compose".into(),
        "-p".into(),
        compose_project_name(worktree_path),
    ];
    for f in &compose.docker_compose_file {
        args.push("-f".into());
        args.push(
            worktree_path
                .join(".devcontainer")
                .join(f)
                .to_string_lossy()
                .into_owned(),
        );
    }
    if let Some(override_file) = override_file {
        args.push("-f".into());
        args.push(override_file.to_string_lossy().into_owned());
    }
    args
}

/// Write the compose override and return docker compose base args.
pub fn compose_args(
    devcontainer: &Devcontainer,
    compose: &Compose,
    worktree_path: &Path,
    project_name: &str,
    project: &Project,
) -> eyre::Result<Vec<String>> {
    let dc_options = &devcontainer.common.customizations.devconcurrent;
    let config_file = worktree_path
        .join(".devcontainer")
        .join("devcontainer.json");
    let override_file = write_compose_override(
        devcontainer,
        worktree_path,
        &config_file,
        project_name,
        dc_options.mount_git,
        project,
    )?;
    Ok(compose_base_args(
        compose,
        worktree_path,
        Some(&override_file),
    ))
}

pub fn compose_up_args(compose: &Compose, base_args: &[String]) -> vec1::Vec1<String> {
    let mut args = vec1::vec1!["docker".into()];
    args.extend(base_args.iter().cloned());
    args.extend(["up".into(), "-d".into(), "--build".into()]);

    if let Some(ref services) = compose.run_services {
        let mut to_start: Vec<String> = services.clone();
        if !to_start.contains(&compose.service) {
            to_start.push(compose.service.clone());
        }
        args.extend(to_start);
    }

    args
}

pub async fn compose_ps_q(compose: &Compose, base_args: &[String]) -> eyre::Result<String> {
    let mut args: Vec<String> = base_args.to_vec();
    args.extend(["ps".into(), "-q".into(), compose.service.clone()]);

    let out = tokio::process::Command::new("docker")
        .args(&args)
        .output()
        .await?;
    eyre::ensure!(out.status.success(), "docker compose ps failed");
    let output = String::from_utf8(out.stdout)?;
    let id = output.lines().next().unwrap_or("").trim().to_string();
    if id.is_empty() {
        return Err(eyre!(
            "no container found for service '{}'",
            compose.service
        ));
    }
    Ok(id)
}

/// Generate a compose override file with:
/// * Our own identification labels
/// * Devcontainer standard labels
/// * Other devcontainer overrides
fn write_compose_override(
    devcontainer: &Devcontainer,
    worktree_path: &Path,
    config_file: &Path,
    project_name: &str,
    mount_git: bool,
    project: &Project,
) -> eyre::Result<PathBuf> {
    let override_path = std::env::temp_dir().join(format!(
        "{}-override.yml",
        compose_project_name(worktree_path)
    ));
    let local_folder = worktree_path.display();
    let config_file = config_file.display();

    let mut service_obj = json!({
        "labels": [
            format!("devcontainer.local_folder={local_folder}"),
            format!("devcontainer.config_file={config_file}"),
            "dev.dc.managed=true".to_string(),
            format!("dev.dc.project={project_name}"),
        ]
    });

    let mut env = project.environment.clone();
    env.extend(
        devcontainer
            .common
            .container_env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    if !env.is_empty() {
        service_obj["environment"] = json!(env);
    }

    if let Some(init) = devcontainer.common.init {
        service_obj["init"] = json!(init);
    }
    if let Some(privileged) = devcontainer.common.privileged {
        service_obj["privileged"] = json!(privileged);
    }
    if !devcontainer.common.cap_add.is_empty() {
        service_obj["cap_add"] = json!(devcontainer.common.cap_add);
    }
    if !devcontainer.common.security_opt.is_empty() {
        service_obj["security_opt"] = json!(devcontainer.common.security_opt);
    }
    if let Some(ref user) = devcontainer.common.container_user {
        service_obj["user"] = json!(user);
    }

    let mut volumes = project.volumes.clone();
    if mount_git && worktree_path != project.path {
        let git_dir = project.path.join(".git");
        volumes.push(format!("{}:{}", git_dir.display(), git_dir.display()));
    }
    if !volumes.is_empty() {
        service_obj["volumes"] = json!(volumes);
    }

    if devcontainer.compose().override_command {
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

    std::fs::write(&override_path, content)
        .wrap_err_with(|| format!("failed to write {}", override_path.display()))?;
    Ok(override_path)
}
