//! `dc proxy` subcommand: lifecycle of the global proxy container plus the
//! per-project config files in its shared volume.

use std::net::IpAddr;
use std::process::Command;
use std::time::Duration;

use clap::{Args, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use docker::Docker;
use eyre::{Result, WrapErr};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};
use shared::{
    ENV_BIND_ADDRESS, ENV_DNS_PORT, MANAGED_LABEL, PROXY_CONFIG_DIR, PROXY_CONFIG_HASH_LABEL,
    PROXY_CONFIG_VOLUME, PROXY_CONTAINER_NAME, PROXY_LABEL, PROXY_SIDECAR_LABEL, PROXY_SOCKS_DIR,
    PROXY_SOCKS_VOLUME, PortMapping, ProjectProxyConfig, ServiceConfig,
};

use crate::state::State;

/// OCI image used by the proxy container. Tag is the CLI's own crate version.
const PROXY_IMAGE: &str = "ghcr.io/paholg/devconcurrent-proxy";

#[derive(Debug, Args)]
pub(crate) struct Proxy {
    #[command(subcommand)]
    command: ProxyCommands,
}

#[derive(Debug, Subcommand)]
enum ProxyCommands {
    /// Start or restart the proxy at the CLI's version and push the current
    /// project's config.
    Up,
    /// Stop and remove the proxy container and all of its sidecars.
    Down,
    /// Show the proxy container's state and registered projects.
    Status,
}

impl Proxy {
    pub(crate) async fn run(self, state: State) -> Result<()> {
        match self.command {
            ProxyCommands::Up => proxy_up(&state).await,
            ProxyCommands::Down => down(&state).await,
            ProxyCommands::Status => status(&state).await,
        }
    }
}

/// Inputs that define the proxy container's *runtime config*. Two containers
/// with the same plan are interchangeable; differing in any field means the
/// existing container is stale and must be recreated.
#[derive(Debug)]
struct ProxyContainerPlan {
    image_ref: String,
    network_mode: &'static str,
    binds: Vec<String>,
    env: Vec<String>,
}

impl ProxyContainerPlan {
    fn build(state: &State, docker: &Docker) -> Self {
        let image_ref = format!("{}:{}", PROXY_IMAGE, env!("CARGO_PKG_VERSION"));
        let socket_path = docker.socket().display().to_string();
        let binds = vec![
            format!("{PROXY_SOCKS_VOLUME}:{PROXY_SOCKS_DIR}"),
            format!("{PROXY_CONFIG_VOLUME}:{PROXY_CONFIG_DIR}"),
            format!("{socket_path}:/var/run/docker.sock"),
        ];
        let env = vec![
            format!("{ENV_BIND_ADDRESS}={}", state.proxy.bind_address),
            format!("{ENV_DNS_PORT}={}", state.proxy.port),
        ];
        Self {
            image_ref,
            network_mode: "host",
            binds,
            env,
        }
    }

    /// Stable hex sha256 over the plan's fields. Sorts list-typed fields so
    /// order-of-insertion changes don't trigger spurious recreates.
    fn hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.image_ref.as_bytes());
        h.update([0]);
        h.update(self.network_mode.as_bytes());
        h.update([0]);
        let mut binds: Vec<&str> = self.binds.iter().map(String::as_str).collect();
        binds.sort();
        for b in binds {
            h.update(b.as_bytes());
            h.update([0]);
        }
        let mut env: Vec<&str> = self.env.iter().map(String::as_str).collect();
        env.sort();
        for e in env {
            h.update(e.as_bytes());
            h.update([0]);
        }
        hex_encode(&h.finalize())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// `dc proxy up` always recreates: the user's intent on invocation is "restart
/// the proxy with current state", whether or not the existing one looks
/// healthy.
async fn proxy_up(state: &State) -> Result<()> {
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    let plan = ProxyContainerPlan::build(state, &docker);
    ensure_loopback_alias(state.proxy.bind_address)?;
    docker
        .ensure_image(&plan.image_ref)
        .await
        .wrap_err_with(|| format!("pull {}", plan.image_ref))?;
    recreate_with_config(state, &docker, &plan).await?;
    log_result(state);
    Ok(())
}

/// `dc up` path: leave a healthy proxy alone, recreate only if missing,
/// stopped, or whose stamped config-hash label doesn't match what we'd build now.
pub(crate) async fn ensure_up(state: &State) -> Result<()> {
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    let plan = ProxyContainerPlan::build(state, &docker);
    ensure_loopback_alias(state.proxy.bind_address)?;
    docker
        .ensure_image(&plan.image_ref)
        .await
        .wrap_err_with(|| format!("pull {}", plan.image_ref))?;
    if proxy_healthy(&docker, &plan).await? {
        push_config_if_any(&docker, state).await?;
    } else {
        recreate_with_config(state, &docker, &plan).await?;
    }
    log_result(state);
    Ok(())
}

fn log_result(state: &State) {
    if let Ok(Some(cfg)) = build_project_config(state) {
        eprintln!(
            "{} proxy is up; pushed config for project {}",
            "✓".green(),
            cfg.project
        );
    } else {
        eprintln!(
            "{} proxy is up; project {} has no proxy config to push",
            "✓".green(),
            state.project_name
        );
    }
}

/// Force-remove any existing proxy container, create a fresh one (stopped),
/// push the project's config into its volume, then start it. Doing the push
/// before `start` means `bootstrap_running_workspaces` (which runs early on
/// proxy startup) sees the config and can adopt any matching workspace
/// containers that were already running.
async fn recreate_with_config(
    state: &State,
    docker: &Docker,
    plan: &ProxyContainerPlan,
) -> Result<()> {
    remove_existing_proxy(docker).await?;
    let id = create_proxy_stopped(docker, plan).await?;
    push_config_if_any(docker, state).await?;
    docker
        .start_container(&id)
        .await
        .wrap_err("start proxy container")?;
    // Give the daemon a moment to flip the container to running.
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}

async fn proxy_healthy(docker: &Docker, plan: &ProxyContainerPlan) -> Result<bool> {
    let want_hash = plan.hash();
    match docker.inspect_container(PROXY_CONTAINER_NAME).await {
        Ok(d) => Ok(d.state.running
            && d.config.image == plan.image_ref
            && d.config
                .labels
                .get(PROXY_CONFIG_HASH_LABEL)
                .map(String::as_str)
                == Some(want_hash.as_str())),
        Err(docker::Error::NotFound) => Ok(false),
        Err(e) => Err(e).wrap_err("inspect existing proxy container"),
    }
}

async fn remove_existing_proxy(docker: &Docker) -> Result<()> {
    match docker
        .remove_container(PROXY_CONTAINER_NAME)
        .force(true)
        .call()
        .await
    {
        Ok(()) | Err(docker::Error::NotFound) => Ok(()),
        Err(e) => Err(e).wrap_err("remove stale proxy container"),
    }
}

async fn create_proxy_stopped(docker: &Docker, plan: &ProxyContainerPlan) -> Result<String> {
    let mut builder = docker
        .create_container(PROXY_CONTAINER_NAME)
        .image(&plan.image_ref)
        .network_mode(plan.network_mode)
        .with_label(MANAGED_LABEL, "true")
        .with_label(PROXY_LABEL, "true")
        .with_label(PROXY_CONFIG_HASH_LABEL, plan.hash());
    for bind in &plan.binds {
        builder = builder.with_bind(bind);
    }
    for entry in &plan.env {
        let (k, v) = entry.split_once('=').expect("plan env is KEY=VALUE");
        builder = builder.with_env(k, v);
    }
    builder.call().await.wrap_err("create proxy container")
}

async fn push_config_if_any(docker: &Docker, state: &State) -> Result<()> {
    if let Some(cfg) = build_project_config(state)? {
        push_config(docker, &cfg).await?;
    }
    Ok(())
}

async fn down(state: &State) -> Result<()> {
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    // Remove the proxy container first; that's enough to halt new sidecar
    // creates. Then sweep sidecars.
    match docker
        .remove_container(PROXY_CONTAINER_NAME)
        .force(true)
        .call()
        .await
    {
        Ok(()) | Err(docker::Error::NotFound) => {}
        Err(e) => return Err(e).wrap_err("remove proxy container"),
    }
    let sidecars = docker
        .list_containers()
        .all(true)
        .with_label(PROXY_SIDECAR_LABEL, "true")
        .call()
        .await
        .wrap_err("list proxy sidecars")?;
    for sc in sidecars {
        match docker.remove_container(&sc.id).force(true).call().await {
            Ok(()) | Err(docker::Error::NotFound) => {}
            Err(e) => tracing::warn!(id = %sc.id, "remove sidecar: {e}"),
        }
    }
    let _ = state; // currently unused
    eprintln!("{} proxy stopped", "✓".green());
    Ok(())
}

async fn status(state: &State) -> Result<()> {
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    match docker.inspect_container(PROXY_CONTAINER_NAME).await {
        Ok(d) => {
            println!(
                "proxy container: {} ({}) image={}",
                d.state.status,
                if d.state.running {
                    "running"
                } else {
                    "stopped"
                },
                d.config.image
            );
        }
        Err(docker::Error::NotFound) => {
            println!("proxy container: not present");
            return Ok(());
        }
        Err(e) => return Err(e).wrap_err("inspect proxy"),
    }
    println!(
        "proxy bind: {}:{}",
        state.proxy.bind_address, state.proxy.port
    );
    Ok(())
}

fn build_project_config(state: &State) -> Result<Option<ProjectProxyConfig>> {
    let Some(dc) = state.devcontainer.as_ref() else {
        return Ok(None);
    };
    let Some(opts) = dc.config.customizations.devconcurrent.proxy.as_ref() else {
        return Ok(None);
    };
    if opts.services.is_empty() && dc.config.forward_ports.is_empty() {
        return Ok(None);
    }

    let domain_template = opts
        .domain_name
        .as_ref()
        .map(|t| t.source().to_string())
        .unwrap_or_else(|| crate::devcontainer::proxy::DEFAULT_TEMPLATE.to_string());

    let mut services_map: IndexMap<String, Vec<PortMapping>> = IndexMap::new();
    for (name, svc) in &opts.services {
        for port in &svc.ports {
            services_map
                .entry(name.clone())
                .or_default()
                .push(PortMapping {
                    ip: port.ip,
                    host: port.host,
                    container: port.container,
                });
        }
    }
    for fp in &dc.config.forward_ports {
        let svc_name = fp
            .service
            .clone()
            .unwrap_or_else(|| dc.config.service.clone());
        let entry = services_map.entry(svc_name).or_default();
        if !entry.iter().any(|p| p.container == fp.port) {
            entry.push(PortMapping {
                ip: state.proxy.bind_address,
                host: fp.port,
                container: fp.port,
            });
        }
    }

    let services: Vec<ServiceConfig> = services_map
        .into_iter()
        .map(|(name, ports)| ServiceConfig { name, ports })
        .collect();

    Ok(Some(ProjectProxyConfig {
        project: state.project_name.to_string(),
        devcontainer_service: dc.config.service.clone(),
        domain_template,
        services,
    }))
}

async fn push_config(docker: &Docker, cfg: &ProjectProxyConfig) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(cfg).wrap_err("serialize project config")?;
    let filename = format!("{}.json", cfg.project);
    let tar = docker::build_single_file_tar(&filename, &bytes);
    docker
        .upload_archive(PROXY_CONTAINER_NAME, PROXY_CONFIG_DIR, tar)
        .await
        .wrap_err("upload project config to proxy")?;
    Ok(())
}

fn ensure_loopback_alias(addr: IpAddr) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    if alias_present_macos(addr)? {
        return Ok(());
    }
    eprintln!(
        "{} {addr} not aliased on lo0; running `sudo ifconfig lo0 alias {addr} up`",
        "?".yellow()
    );
    let status = Command::new("sudo")
        .args(["ifconfig", "lo0", "alias", &addr.to_string(), "up"])
        .status()
        .wrap_err("invoke sudo ifconfig")?;
    if !status.success() {
        eyre::bail!("failed to add loopback alias for {addr}");
    }
    print_macos_persistence_hint(addr);
    Ok(())
}

fn alias_present_macos(addr: IpAddr) -> Result<bool> {
    let out = Command::new("ifconfig")
        .arg("lo0")
        .output()
        .wrap_err("run ifconfig lo0")?;
    if !out.status.success() {
        eyre::bail!(
            "ifconfig lo0 failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.contains(&addr.to_string()))
}

fn print_macos_persistence_hint(addr: IpAddr) {
    let plist_path = "/Library/LaunchDaemons/dev.devconcurrent.lo0-alias.plist";
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.devconcurrent.lo0-alias</string>
  <key>RunAtLoad</key><true/>
  <key>ProgramArguments</key>
  <array>
    <string>/sbin/ifconfig</string>
    <string>lo0</string>
    <string>alias</string>
    <string>{addr}</string>
    <string>up</string>
  </array>
</dict>
</plist>"#
    );
    eprintln!();
    eprintln!("To make this loopback alias survive reboots, save the following as");
    eprintln!("  {plist_path}");
    eprintln!("then run:");
    eprintln!("  sudo launchctl bootstrap system {plist_path}");
    eprintln!();
    eprintln!("---8<---");
    eprintln!("{plist}");
    eprintln!("--->8---");
}
