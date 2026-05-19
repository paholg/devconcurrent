use std::sync::LazyLock;
use std::time::Duration;

use clap::{Args, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use docker::Docker;
use eyre::{Result, WrapErr};
use indexmap::IndexMap;
use shared::{
    ENV_CA_DIR, ENV_DNS_PORT, MANAGED_LABEL, PROXY_CA_DIR, PROXY_CONFIG_DIR, PROXY_CONFIG_VOLUME,
    PROXY_CONTAINER_NAME, PROXY_GROUP_LABEL, PROXY_LABEL, PortMapping, ProjectProxyConfig,
    ServiceConfig,
};

use crate::config::{Config, Project, ProjectName};
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
    println!("proxy dns port: {}", config.proxy.port);

    for (name, project) in &config.projects {
        let Some(cfg) = build_project_config(name, project)? else {
            continue;
        };
        println!();
        println!("project: {name}");
        for svc in &cfg.services {
            let ports = svc
                .ports
                .iter()
                .map(|p| {
                    let tls = if p.tls { " (tls)" } else { "" };
                    format!("{}->{}{tls}", p.host, p.container)
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!("  - {}: {}", svc.name, ports);
        }
    }
    Ok(())
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
        .with_label(MANAGED_LABEL, "true")
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
    for (name, project) in &config.projects {
        let Some(cfg) = build_project_config(name, project)? else {
            continue;
        };
        push_config(docker, &cfg).await?;
        eprintln!("{} pushed config for {name}", "✓".green());
    }
    Ok(())
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

fn build_project_config(
    name: &ProjectName,
    project: &Project,
) -> Result<Option<ProjectProxyConfig>> {
    let dc_path = DevcontainerConfig::find_config(&project.path);
    let Some(dc_config) = DevcontainerConfig::load(dc_path.as_deref(), project)? else {
        return Ok(None);
    };
    let opts = &dc_config.customizations.devconcurrent.proxy;
    if !opts.enable {
        return Ok(None);
    }

    let domain_template = opts
        .domain_name
        .as_ref()
        .map(|t| t.source().to_string())
        .unwrap_or_else(|| crate::devcontainer::proxy_options::DEFAULT_TEMPLATE.to_string());

    let mut services_map: IndexMap<String, Vec<PortMapping>> = IndexMap::new();
    for (svc_name, svc) in &opts.services {
        for port in &svc.ports {
            if port.tls && port.host == port.container {
                eyre::bail!(
                    "project {name}: service {svc_name:?} tls port {}:{} has host == container; \
                     TLS termination requires a distinct host port",
                    port.host,
                    port.container,
                );
            }
            services_map
                .entry(svc_name.clone())
                .or_default()
                .push(PortMapping {
                    host: port.host,
                    container: port.container,
                    tls: port.tls,
                });
        }
    }
    let services: Vec<ServiceConfig> = services_map
        .into_iter()
        .map(|(name, ports)| ServiceConfig { name, ports })
        .collect();

    Ok(Some(ProjectProxyConfig {
        project: name.to_string(),
        domain_template,
        services,
    }))
}
