use bollard::Docker;
use clap::Args;

use crate::{config::Config, workspace::Workspace};

/// List active devcontainers
#[derive(Debug, Args)]
pub struct List {
    #[arg(
        short,
        long,
        help = "name of project [default: The first one configured]"
    )]
    project: Option<Option<String>>,
}

impl List {
    pub async fn run(self, docker: &Docker, config: &Config) -> eyre::Result<()> {
        dbg!(Workspace::list_all(docker, config).await?);
        Ok(())
    }
}
