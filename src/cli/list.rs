use bollard::Docker;
use clap::Args;

use crate::config::Config;
use crate::workspace::{Workspace, workspace_table};

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
        let project = self.project.as_ref().and_then(|p| p.as_deref());
        let workspaces = Workspace::list_project(docker, project, config).await?;
        print!("{}", workspace_table(&workspaces));
        Ok(())
    }
}
