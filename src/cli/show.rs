use clap::{Args, Subcommand};
use itertools::Itertools;

use crate::{cli::State, cli::fwd};

/// Show some value
#[derive(Debug, Args)]
pub(crate) struct Show {
    #[command(subcommand)]
    command: ShowCommands,
}

#[derive(Debug, Subcommand)]
enum ShowCommands {
    /// Show currently-forwarded ports for this workspace
    Ports(Ports),
    /// Print the current workspace name, or exit 1
    Workspace(ShowWorkspace),
    /// Show container IP addresses for this workspace
    Ip(Ip),
}

#[derive(Debug, Args)]
struct Ports;

#[derive(Debug, Args)]
struct ShowWorkspace;

#[derive(Debug, Args)]
struct Ip {
    /// Compose service name; if omitted, list all services for this workspace
    service: Option<String>,
}

impl Show {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        match self.command {
            ShowCommands::Ports(ports) => ports.run(state).await,
            ShowCommands::Workspace(ws) => ws.run(state).await,
            ShowCommands::Ip(ip) => ip.run(state).await,
        }
    }
}

impl Ports {
    async fn run(self, state: State) -> eyre::Result<()> {
        let ports = get_ports(state).await?;

        println!("{ports}");
        Ok(())
    }
}

async fn get_ports(state: State) -> eyre::Result<String> {
    let workspace = state.resolve_workspace(None).await?;
    let devcontainer = state.try_devcontainer()?;
    let (ports, healthy) = tokio::join!(
        devcontainer.docker.workspace_forwarded_ports(&workspace),
        devcontainer.docker.is_forwarding_healthy(&workspace),
    );
    let ports = ports?;

    if !ports.is_empty() && !healthy? {
        fwd::remove_sidecars(&state).await?;
        Ok(String::new())
    } else {
        Ok(ports.into_iter().join(","))
    }
}

impl ShowWorkspace {
    async fn run(self, state: State) -> eyre::Result<()> {
        match state.resolve_workspace(None).await {
            Ok(workspace) => {
                println!("{}", workspace.name);
                Ok(())
            }
            Err(_) => std::process::exit(1),
        }
    }
}

impl Ip {
    async fn run(self, state: State) -> eyre::Result<()> {
        let devcontainer = state.try_devcontainer()?;
        let workspace = state.resolve_workspace(None).await?;
        let ips = devcontainer
            .docker
            .workspace_compose_ips(&workspace)
            .await?;

        if let Some(service) = self.service {
            let ip = ips.iter().find(|(s, _)| s == &service).ok_or_else(|| {
                eyre::eyre!(
                    "no service '{service}' with an IP address in workspace '{}'",
                    workspace.name
                )
            })?;
            println!("{}", ip.1);
        } else {
            for (service, ip) in ips {
                println!("{service}\t{ip}");
            }
        }
        Ok(())
    }
}
