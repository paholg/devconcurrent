use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;
use std::time::Duration;

use clap::{Args, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use comfy_table::{Cell, Color, ContentArrangement, Table, presets};
use docker::{
    ContainerStatus, Docker, PROJECT_LABEL, PROXY_GROUP_LABEL, PROXY_LABEL, PROXY_SERVICE_LABEL,
    PROXY_SIDECAR_LABEL, WORKSPACE_LABEL,
};
use eyre::{Result, WrapErr};
use shared::{
    ENV_CA_DIR, ENV_DNS_PORT, PROXY_CA_DIR, PROXY_CONFIG_DIR, PROXY_CONFIG_FILE,
    PROXY_CONFIG_VOLUME, PROXY_CONTAINER_NAME, ProxyOptions, ProxyService,
};

use crate::config::{Config, Project};
use crate::devcontainer::DevcontainerConfig;

/// OCI image used by the proxy container.
const PROXY_IMAGE_NAME: &str = "ghcr.io/paholg/devconcurrent-proxy";
/// We keep the proxy and CLI versions in sync, so using the CLI version here is fine.
const PROXY_IMAGE_TAG: &str = env!("CARGO_PKG_VERSION");

static PROXY_IMAGE: LazyLock<String> =
    LazyLock::new(|| format!("{PROXY_IMAGE_NAME}:{PROXY_IMAGE_TAG}"));

#[derive(Debug, Args)]
pub(crate) struct Proxy {
    #[command(subcommand)]
    command: ProxyCommands,
}

#[derive(Debug, Subcommand)]
enum ProxyCommands {
    /// Start or restart the proxy
    Up,
    /// Stop and remove the proxy
    Down,
    /// View the current proxy status
    Status,
}

impl Proxy {
    pub(crate) async fn run(self) -> Result<()> {
        match self.command {
            ProxyCommands::Up => proxy_up().await,
            ProxyCommands::Down => proxy_down().await,
            ProxyCommands::Status => proxy_status().await,
        }
    }
}

/// `dc proxy up`: force-remove the proxy and every sidecar, then create a
/// fresh proxy and push every proxy-enabled project's config into its
/// volume. The new proxy's bootstrap creates fresh sidecars for any running
/// workspaces.
async fn proxy_up() -> Result<()> {
    let config = Config::load()?;
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    docker
        .ensure_image(&PROXY_IMAGE)
        .await
        .wrap_err_with(|| format!("pull {}", *PROXY_IMAGE))?;
    remove_proxy_group(&docker).await?;
    let id = create_proxy_stopped(&config, &docker).await?;
    push_all_configs(&config, &docker).await?;
    docker
        .start_container(&id)
        .await
        .wrap_err("start proxy container")?;
    wait_for_running(&docker, &id).await?;
    eprintln!("{} proxy is running", "✓".green());
    Ok(())
}

/// Force-remove every container the proxy owns — the proxy itself plus its
/// sidecars. They all carry `PROXY_GROUP_LABEL`, so one `list_containers`
/// returns the whole set.
async fn remove_proxy_group(docker: &Docker) -> Result<()> {
    let members = docker
        .list_containers()
        .all(true)
        .with_label(PROXY_GROUP_LABEL, "true")
        .call()
        .await
        .wrap_err("list proxy group")?;
    for c in members {
        match docker.remove_container(&c.id).force(true).call().await {
            Ok(()) | Err(docker::Error::NotFound) => {}
            Err(e) => tracing::warn!(id = %c.id, "remove proxy-group container: {e}"),
        }
    }
    Ok(())
}

/// `dc up` path: ensure the proxy is up. If the container is already running,
/// leave it alone (we don't try to detect drift here — `dc proxy up` is the
/// explicit refresh). Otherwise bring it up fresh.
pub(crate) async fn ensure_up() -> Result<()> {
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    let running = match docker.inspect_container(PROXY_CONTAINER_NAME).await {
        Ok(d) => d.state.running,
        Err(docker::Error::NotFound) => false,
        Err(e) => return Err(e).wrap_err("inspect proxy"),
    };
    if running {
        return Ok(());
    }
    proxy_up().await
}

async fn proxy_down() -> Result<()> {
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    remove_proxy_group(&docker).await?;
    eprintln!("{} proxy stopped", "✓".green());
    Ok(())
}

async fn proxy_status() -> Result<()> {
    let config = Config::load()?;
    let docker = Docker::connect().await.wrap_err("connect to docker")?;
    match docker.inspect_container(PROXY_CONTAINER_NAME).await {
        Ok(d) if d.state.running => {
            println!(
                "proxy: {} (image={}, dns port={})",
                "running".green(),
                d.config.image,
                config.proxy.port,
            );
        }
        Ok(d) => {
            println!(
                "proxy: {} ({}, image={})",
                "not running".red(),
                d.state.status,
                d.config.image,
            );
        }
        Err(docker::Error::NotFound) => {
            println!("proxy: {}", "not present".red());
            return Ok(());
        }
        Err(e) => return Err(e).wrap_err("inspect proxy"),
    }

    let mut proxy_options: HashMap<String, ProxyOptions> = HashMap::new();
    for (name, project) in &config.projects {
        if let Some(opts) = load_proxy_options(project)? {
            proxy_options.insert(name.to_string(), opts);
        }
    }

    let sidecars = docker
        .list_containers()
        .all(true)
        .with_label(PROXY_SIDECAR_LABEL, "true")
        .call()
        .await
        .wrap_err("list sidecars")?;

    if sidecars.is_empty() {
        println!();
        println!("no sidecars running");
        return Ok(());
    }

    // project -> workspace -> sorted service rows
    let mut grouped: BTreeMap<String, BTreeMap<String, Vec<ServiceRow>>> = BTreeMap::new();
    for sc in sidecars {
        let project = sc.labels.get(PROJECT_LABEL).cloned().unwrap_or_default();
        let workspace = sc.labels.get(WORKSPACE_LABEL).cloned().unwrap_or_default();
        let service = sc
            .labels
            .get(PROXY_SERVICE_LABEL)
            .cloned()
            .unwrap_or_default();
        let opts = proxy_options.get(&project);
        let svc_cfg = opts.and_then(|o| o.services.get(&service)).cloned();
        let domain = opts
            .and_then(|o| o.render_hostname(&project, &workspace, &service, workspace == project));
        grouped
            .entry(project)
            .or_default()
            .entry(workspace)
            .or_default()
            .push(ServiceRow {
                service,
                domain,
                proxy: sc.state,
                container_id: sc.id,
                ports: svc_cfg,
            });
    }
    for workspaces in grouped.values_mut() {
        for rows in workspaces.values_mut() {
            rows.sort_by(|a, b| a.service.cmp(&b.service));
        }
    }

    for (project, workspaces) in &grouped {
        println!();
        println!("project: {}", project.bold());
        println!("{}", service_table(workspaces));
    }
    Ok(())
}

struct ServiceRow {
    service: String,
    domain: Option<String>,
    proxy: ContainerStatus,
    container_id: String,
    ports: Option<ProxyService>,
}

fn service_table(workspaces: &BTreeMap<String, Vec<ServiceRow>>) -> Table {
    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header([
            "WORKSPACE",
            "SERVICE",
            "DOMAIN",
            "STATUS",
            "CONTAINER",
            "PORTS",
        ]);
    for (workspace, rows) in workspaces {
        for (i, row) in rows.iter().enumerate() {
            let workspace_cell = if i == 0 { workspace.as_str() } else { "" };
            table.add_row([
                Cell::new(workspace_cell),
                Cell::new(&row.service),
                domain_cell(row.domain.as_deref()),
                status_cell(row.proxy),
                Cell::new(short_id(&row.container_id)),
                ports_cell(row.ports.as_ref()),
            ]);
        }
    }
    table
}

fn domain_cell(domain: Option<&str>) -> Cell {
    match domain {
        Some(d) if !d.is_empty() => Cell::new(d),
        _ => Cell::new("-").fg(Color::DarkGrey),
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn status_cell(status: ContainerStatus) -> Cell {
    let cell = Cell::new(status);
    match status {
        ContainerStatus::Running => cell.fg(Color::Green),
        ContainerStatus::Exited | ContainerStatus::Dead => cell.fg(Color::Red),
        _ => cell.fg(Color::Yellow),
    }
}

fn ports_cell(svc: Option<&ProxyService>) -> Cell {
    let Some(svc) = svc else {
        return Cell::new("-").fg(Color::DarkGrey);
    };
    if svc.ports.is_empty() {
        return Cell::new("-").fg(Color::DarkGrey);
    }
    let text = svc
        .ports
        .iter()
        .map(|p| p.host.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Cell::new(text)
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

async fn create_proxy_stopped(config: &Config, docker: &Docker) -> Result<String> {
    let socket_path = docker.socket().display();
    let mut builder = docker
        .create_container(PROXY_CONTAINER_NAME)
        .image(&PROXY_IMAGE)
        .network_mode("host")
        .with_label(PROXY_LABEL, "true")
        .with_label(PROXY_GROUP_LABEL, "true")
        .with_bind(PROXY_CONFIG_VOLUME, PROXY_CONFIG_DIR)
        .with_bind(socket_path, "/var/run/docker.sock")
        .with_env(ENV_DNS_PORT, config.proxy.port);

    if let Some(ca_root) = &config.proxy.ca_root {
        builder = builder
            .with_ro_bind(ca_root.display(), PROXY_CA_DIR)
            .with_env(ENV_CA_DIR, PROXY_CA_DIR);
    }

    builder.call().await.wrap_err("create proxy container")
}

async fn push_all_configs(config: &Config, docker: &Docker) -> Result<()> {
    let mut all: HashMap<String, ProxyOptions> = HashMap::new();
    for (name, project) in &config.projects {
        let Some(opts) = load_proxy_options(project)? else {
            continue;
        };
        all.insert(name.to_string(), opts);
    }
    let bytes = serde_json::to_vec_pretty(&all).wrap_err("serialize proxy projects")?;
    let tar = docker::build_single_file_tar(PROXY_CONFIG_FILE, &bytes);
    docker
        .upload_archive(PROXY_CONTAINER_NAME, PROXY_CONFIG_DIR, tar)
        .await
        .wrap_err("upload proxy projects")?;
    eprintln!(
        "{} pushed config for {} project(s): {}",
        "✓".green(),
        all.len(),
        all.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    Ok(())
}

/// Load and merge this project's devcontainer config and return its
/// `ProxyOptions` if proxying is enabled. Returns `None` for projects with no
/// devcontainer.json, or with `proxy.enable = false`.
fn load_proxy_options(project: &Project) -> Result<Option<ProxyOptions>> {
    let dc_path = DevcontainerConfig::find_config(&project.path);
    let Some(dc_config) = DevcontainerConfig::load(dc_path.as_deref(), project)? else {
        return Ok(None);
    };
    let opts = &dc_config.customizations.devconcurrent.proxy;
    if !opts.enable {
        return Ok(None);
    }
    Ok(Some(opts.clone()))
}
