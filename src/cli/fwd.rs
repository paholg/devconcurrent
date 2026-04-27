use clap::{Args, Subcommand};
use clap_complete::ArgValueCompleter;
use eyre::eyre;
use tokio::process::Command;

use color_eyre::owo_colors::OwoColorize;

use crate::cli::State;
use crate::complete::complete_workspace;
use crate::devcontainer::forward_port::ForwardPort;
use crate::state::DevcontainerState;
use crate::workspace::{Workspace, WorkspaceMini};

const SOCAT_IMAGE: &str = "docker.io/alpine/socat:latest";

/// Forward configured `forwardPorts` to a running workspace
#[derive(Debug, Args)]
pub struct Fwd {
    /// Workspace name [default: current working directory]
    #[arg(short, long, add = ArgValueCompleter::new(complete_workspace))]
    workspace: Option<String>,

    #[command(subcommand)]
    command: Option<FwdCommands>,
}

#[derive(Debug, Subcommand)]
enum FwdCommands {
    /// Stop forwarding ports (remove sidecar containers)
    Stop,
}

impl Fwd {
    pub async fn run(self, state: State) -> eyre::Result<()> {
        let devcontainer = state.try_devcontainer()?;
        match self.command {
            Some(FwdCommands::Stop) => remove_sidecars(&state).await,
            None => {
                let workspace = state.resolve_workspace(self.workspace).await?;
                forward(&state, devcontainer, &workspace).await
            }
        }
    }
}

pub async fn forward(
    state: &State,
    devcontainer: &DevcontainerState,
    workspace: &WorkspaceMini,
) -> eyre::Result<()> {
    remove_sidecars(state).await?;

    let ws = Workspace::get(state, devcontainer, &workspace.name).await?;
    let cid = ws.service_container_id()?;
    let dc = state.try_devcontainer()?;
    let ports = &dc.config.common.forward_ports;

    if ports.is_empty() {
        return Ok(());
    }

    let free: Vec<bool> = ports.iter().map(|p| port_is_free(p.port)).collect();
    let available: Vec<ForwardPort> = ports
        .iter()
        .zip(&free)
        .filter(|(_, ok)| **ok)
        .map(|(p, _)| p.clone())
        .collect();

    if !available.is_empty() {
        // Get container's network name for the outer sidecar
        let network_name = container_network(cid).await?;

        ensure_image().await?;

        let volume_name = format!("devconcurrent-fwd-{}", ws.compose_project_name);

        docker(&[
            "volume",
            "create",
            "--label",
            "dev.devconcurrent.fwd=true",
            "--label",
            &format!("dev.devconcurrent.project={}", state.project_name),
            "--label",
            &format!("dev.devconcurrent.workspace={}", ws.compose_project_name),
            &volume_name,
        ])
        .await?;

        create_inner_sidecar(
            state,
            &ws.compose_project_name,
            cid,
            &volume_name,
            &available,
        )
        .await?;
        create_outer_sidecar(
            state,
            &ws.compose_project_name,
            cid,
            &network_name,
            &volume_name,
            &available,
        )
        .await?;
    }

    for (port, &ok) in ports.iter().zip(&free) {
        if ok {
            eprintln!("{} {port}", "✓".green());
        } else {
            eprintln!("{} {port} (already in use)", "✗".red());
        }
    }

    Ok(())
}

async fn container_network(cid: &str) -> eyre::Result<String> {
    let out = Command::new("docker")
        .args([
            "inspect",
            "-f",
            "{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}",
            cid,
        ])
        .output()
        .await?;
    eyre::ensure!(out.status.success(), "failed to inspect container {cid}");
    let name = String::from_utf8(out.stdout)?.trim().to_string();
    if name.is_empty() {
        return Err(eyre!("container {cid} has no networks"));
    }
    Ok(name)
}

/// Inner sidecar: shares the target container's network namespace.
/// For each port, listens on a Unix socket and connects to 127.0.0.1:<port>.
async fn create_inner_sidecar(
    state: &State,
    compose_project_name: &str,
    cid: &str,
    volume_name: &str,
    ports: &[ForwardPort],
) -> eyre::Result<()> {
    let name = format!("devconcurrent-fwd-inner-{compose_project_name}");

    let socat_cmds: Vec<String> = ports
        .iter()
        .map(|p| {
            let target = p.service.as_deref().unwrap_or("127.0.0.1");
            format!(
                "socat UNIX-LISTEN:/socks/{}.sock,fork,reuseaddr TCP:{target}:{}",
                p.port, p.port
            )
        })
        .collect();
    let shell_cmd = join_background(&socat_cmds);

    let args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        name.clone(),
        format!("--network=container:{cid}"),
        format!("--volume={volume_name}:/socks"),
        "--label".to_string(),
        "dev.devconcurrent.fwd=true".to_string(),
        "--label".to_string(),
        format!("dev.devconcurrent.project={}", state.project_name),
        "--label".to_string(),
        format!("dev.devconcurrent.workspace={compose_project_name}"),
        "--label".to_string(),
        format!("dev.devconcurrent.fwd.target={cid}"),
        "--entrypoint".to_string(),
        "sh".to_string(),
        SOCAT_IMAGE.to_string(),
        "-c".to_string(),
        shell_cmd,
    ];

    docker(&args.iter().map(|s| s.as_str()).collect::<Vec<_>>()).await?;
    Ok(())
}

/// Outer sidecar: on the Docker network with host port bindings.
/// For each port, listens on TCP and connects via the Unix socket.
async fn create_outer_sidecar(
    state: &State,
    compose_project_name: &str,
    cid: &str,
    network_name: &str,
    volume_name: &str,
    ports: &[ForwardPort],
) -> eyre::Result<()> {
    let name = format!("devconcurrent-fwd-{compose_project_name}");

    let socat_cmds: Vec<String> = ports
        .iter()
        .map(|p| {
            format!(
                "socat TCP-LISTEN:{},fork,reuseaddr UNIX:/socks/{}.sock",
                p.port, p.port
            )
        })
        .collect();
    let shell_cmd = join_background(&socat_cmds);

    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        name.clone(),
        format!("--network={network_name}"),
        format!("--volume={volume_name}:/socks"),
        "--label".to_string(),
        "dev.devconcurrent.fwd=true".to_string(),
        "--label".to_string(),
        format!("dev.devconcurrent.project={}", state.project_name),
        "--label".to_string(),
        format!("dev.devconcurrent.workspace={compose_project_name}"),
        "--label".to_string(),
        format!("dev.devconcurrent.fwd.target={cid}"),
    ];

    for p in ports {
        args.push("-p".to_string());
        args.push(format!("127.0.0.1:{}:{}", p.port, p.port));
    }

    args.push("--entrypoint".to_string());
    args.push("sh".to_string());
    args.push(SOCAT_IMAGE.to_string());
    args.push("-c".to_string());
    args.push(shell_cmd);

    docker(&args.iter().map(|s| s.as_str()).collect::<Vec<_>>()).await?;
    Ok(())
}

/// Build a shell command that runs all socat processes in the background then waits.
fn join_background(cmds: &[String]) -> String {
    let mut parts: Vec<String> = cmds.iter().map(|c| format!("{c} &")).collect();
    parts.push("wait".to_string());
    parts.join(" ")
}

async fn ensure_image() -> eyre::Result<()> {
    let out = Command::new("docker")
        .args(["image", "inspect", SOCAT_IMAGE])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await?;
    if !out.success() {
        docker(&["pull", SOCAT_IMAGE]).await?;
    }
    Ok(())
}

pub async fn remove_sidecars(state: &State) -> eyre::Result<()> {
    let project = &state.project_name;
    let filter = "label=dev.devconcurrent.fwd=true".to_string();
    let filter2 = format!("label=dev.devconcurrent.project={project}");

    let out = Command::new("docker")
        .args(["ps", "-a", "-q", "--filter", &filter, "--filter", &filter2])
        .output()
        .await?;

    let stdout = String::from_utf8(out.stdout)?;
    let ids: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();

    if !ids.is_empty() {
        let mut args = vec!["rm", "-f"];
        args.extend(ids);
        let _ = docker(&args).await;
    }

    // Clean up forwarding volumes
    let out = Command::new("docker")
        .args([
            "volume", "ls", "-q", "--filter", &filter, "--filter", &filter2,
        ])
        .output()
        .await?;
    let stdout = String::from_utf8(out.stdout)?;
    for vol in stdout.lines().filter(|l| !l.is_empty()) {
        let _ = Command::new("docker")
            .args(["volume", "rm", vol])
            .output()
            .await;
    }

    Ok(())
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

async fn docker(args: &[&str]) -> eyre::Result<()> {
    let out = Command::new("docker").args(args).output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(eyre!("docker {} failed: {}", args[0], stderr.trim()));
    }
    Ok(())
}
