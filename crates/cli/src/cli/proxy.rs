//! `dc proxy` subcommand: lifecycle of the global proxy container plus the
//! per-project config files in its shared volume.

use std::time::Duration;

use clap::{Args, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use docker::Docker;
use eyre::{Result, WrapErr};
use handlebars::Handlebars;
use indexmap::IndexMap;
use serde::Serialize;
use sha2::{Digest, Sha256};
use shared::{
    COMPOSE_SERVICE_LABEL, ENV_DNS_PORT, MANAGED_LABEL, PROJECT_LABEL, PROXY_CONFIG_DIR,
    PROXY_CONFIG_HASH_LABEL, PROXY_CONFIG_VOLUME, PROXY_CONTAINER_NAME, PROXY_LABEL,
    PROXY_SIDECAR_LABEL, PortMapping, ProjectProxyConfig, ServiceConfig, WORKSPACE_LABEL,
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
    /// Show the proxy container's state.
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
            format!("{PROXY_CONFIG_VOLUME}:{PROXY_CONFIG_DIR}"),
            format!("{socket_path}:/var/run/docker.sock"),
        ];
        let env = vec![format!("{ENV_DNS_PORT}={}", state.proxy.port)];
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
    wait_for_running(docker, &id).await
}

async fn wait_for_running(docker: &Docker, id: &str) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match docker.inspect_container(id).await {
            Ok(d) if d.state.running => return Ok(()),
            Ok(_) => {}
            Err(docker::Error::NotFound) => eyre::bail!("proxy container vanished after start"),
            Err(e) => return Err(e).wrap_err("inspect proxy after start"),
        }
        if std::time::Instant::now() >= deadline {
            eyre::bail!("proxy container did not reach running state within 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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
    println!("proxy dns port: {}", state.proxy.port);

    let Some(cfg) = build_project_config(state)? else {
        println!();
        println!("project: {}  (no proxy config)", state.project_name);
        return Ok(());
    };

    println!();
    println!("project: {}", cfg.project);
    println!("  configured services:");
    for svc in &cfg.services {
        let ports = svc
            .ports
            .iter()
            .map(|p| format!("{}->{}", p.host, p.container))
            .collect::<Vec<_>>()
            .join(", ");
        println!("    - {}  ports: {}", svc.name, ports);
    }

    let running = list_running_services(&docker, &cfg).await?;
    if running.is_empty() {
        println!("  running workspaces: (none)");
        return Ok(());
    }
    println!("  running workspaces:");
    let mut by_ws: IndexMap<String, Vec<RunningServiceInfo>> = IndexMap::new();
    for r in running {
        by_ws.entry(r.workspace.clone()).or_default().push(r);
    }
    for (workspace, services) in by_ws {
        println!("    {workspace}:");
        for r in services {
            let ip = r.ip.as_deref().unwrap_or("?");
            let hostname = render_hostname(&cfg, &workspace, &r.service)
                .unwrap_or_else(|| "<template error>".to_string());
            println!(
                "      {svc:<12} cid={cid}  ip={ip}  → {hostname}",
                svc = r.service,
                cid = short(&r.cid),
            );
        }
    }
    Ok(())
}

struct RunningServiceInfo {
    workspace: String,
    service: String,
    cid: String,
    ip: Option<String>,
}

async fn list_running_services(
    docker: &Docker,
    cfg: &ProjectProxyConfig,
) -> Result<Vec<RunningServiceInfo>> {
    // Find primaries (containers with the user-set project label).
    let primaries = docker
        .list_containers()
        .with_label(PROJECT_LABEL, &cfg.project)
        .call()
        .await
        .wrap_err("list primary containers")?;
    let mut out = Vec::new();
    let mut seen_compose: std::collections::HashSet<String> = std::collections::HashSet::new();
    for primary in &primaries {
        let Some(compose_project) = primary.labels.get(shared::COMPOSE_PROJECT_LABEL).cloned()
        else {
            continue;
        };
        if !seen_compose.insert(compose_project.clone()) {
            continue;
        }
        let workspace = primary
            .labels
            .get(WORKSPACE_LABEL)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                compose_project
                    .strip_suffix("_devcontainer")
                    .unwrap_or(&compose_project)
                    .to_string()
            });
        let containers = docker
            .list_containers()
            .with_label(shared::COMPOSE_PROJECT_LABEL, &compose_project)
            .call()
            .await
            .wrap_err_with(|| format!("list containers for {compose_project}"))?;
        for c in containers {
            if c.labels.contains_key(shared::PROXY_SIDECAR_LABEL) {
                continue;
            }
            let Some(service) = c.labels.get(COMPOSE_SERVICE_LABEL).cloned() else {
                continue;
            };
            let ip = c
                .network_settings
                .networks
                .values()
                .find_map(|n| n.ip_address.clone())
                .filter(|s| !s.is_empty());
            out.push(RunningServiceInfo {
                workspace: workspace.clone(),
                service,
                cid: c.id,
                ip,
            });
        }
    }
    out.sort_by(|a, b| {
        (a.workspace.as_str(), a.service.as_str()).cmp(&(b.workspace.as_str(), b.service.as_str()))
    });
    Ok(out)
}

#[derive(Serialize)]
struct TemplateContext<'a> {
    root: bool,
    project: &'a str,
    workspace: &'a str,
    service: &'a str,
}

fn render_hostname(cfg: &ProjectProxyConfig, workspace: &str, service: &str) -> Option<String> {
    let mut hbs = Handlebars::new();
    hbs.set_strict_mode(false);
    let ctx = TemplateContext {
        root: workspace == cfg.project,
        project: &cfg.project,
        workspace,
        service,
    };
    hbs.render_template(&cfg.domain_template, &ctx).ok()
}

fn short(cid: &str) -> &str {
    let end = cid.len().min(12);
    &cid[..end]
}

fn build_project_config(state: &State) -> Result<Option<ProjectProxyConfig>> {
    let Some(dc) = state.devcontainer.as_ref() else {
        return Ok(None);
    };
    if !dc.proxy_enabled() {
        return Ok(None);
    }
    let opts = &dc.config.customizations.devconcurrent.proxy;

    let domain_template = opts
        .domain_name
        .as_ref()
        .map(|t| t.source().to_string())
        .unwrap_or_else(|| crate::devcontainer::proxy_options::DEFAULT_TEMPLATE.to_string());

    let mut services_map: IndexMap<String, Vec<PortMapping>> = IndexMap::new();
    for (name, svc) in &opts.services {
        for port in &svc.ports {
            services_map
                .entry(name.clone())
                .or_default()
                .push(PortMapping {
                    host: port.host,
                    container: port.container,
                });
        }
    }
    // `forwardPorts` declares dev-server-on-localhost ports: the app inside
    // the container binds 127.0.0.1:<port> only, so the sidecar can bind
    // 0.0.0.0:<port> in the same netns without colliding and forward to
    // 127.0.0.1:<port>. host == container.
    for fp in &dc.config.forward_ports {
        let svc_name = fp
            .service
            .clone()
            .unwrap_or_else(|| dc.config.service.clone());
        let entry = services_map.entry(svc_name).or_default();
        if !entry.iter().any(|p| p.host == fp.port) {
            entry.push(PortMapping {
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
