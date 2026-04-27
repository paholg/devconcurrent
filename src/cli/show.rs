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
}

#[derive(Debug, Args)]
struct Ports;

#[derive(Debug, Args)]
struct ShowWorkspace;

impl Show {
    pub(crate) async fn run(self, state: State) -> eyre::Result<()> {
        match self.command {
            ShowCommands::Ports(ports) => ports.run(state).await,
            ShowCommands::Workspace(ws) => ws.run(state).await,
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
        devcontainer
            .docker
            .workspace_forwarded_ports(&state, &workspace),
        devcontainer
            .docker
            .is_forwarding_healthy(&state, &workspace),
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
