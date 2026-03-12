use std::env;
use std::io::{BufRead, Write};

use clap::{Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;
use eyre::OptionExt;

use crate::{
    complete,
    config::{Config, Project},
    devcontainer::Devcontainer,
    docker::DockerClient,
    workspace::Workspace,
    worktree,
};

mod archive;
mod compose;
mod copy;
mod destroy;
mod exec;
mod fwd;
mod go;
mod list;
mod show;
mod work;

const ABOUT: &str =
    "A tool for managing devcontainers, especially when combined with git worktrees";

#[derive(Debug, Parser)]
#[command(version, about = ABOUT)]
pub struct Cli {
    #[arg(
        short,
        long,
        help = "name of project [default: The DC_PROJECT variable, then the first configured project]",
        add = ArgValueCompleter::new(complete::complete_project),
    )]
    pub project: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(visible_alias = "w")]
    Work(work::Work),
    #[command(visible_alias = "x")]
    Exec(exec::Exec),
    #[command(visible_alias = "f")]
    Fwd(fwd::Fwd),
    #[command(visible_alias = "l")]
    List(list::List),
    #[command(visible_alias = "c")]
    Compose(compose::Compose),
    #[command()]
    Archive(archive::Archive),
    #[command()]
    Destroy(destroy::Destroy),
    Show(show::Show),
    #[command()]
    Go(go::Go),
}

pub struct State {
    pub docker: DockerClient,
    pub project_name: String,
    pub project: Project,
}

impl State {
    // TODO: We should just load this at start.
    fn devcontainer(&self) -> eyre::Result<Devcontainer> {
        Devcontainer::load(&self.project)
    }

    pub fn is_root(&self, name: &str) -> bool {
        self.project
            .path
            .file_name()
            .is_some_and(|root| name == root)
    }

    /// Find the workspace name.
    ///
    /// If no name is given, or if it's ".", we derive it from the current working direcory.
    pub async fn resolve_workspace(&self, name: Option<String>) -> eyre::Result<String> {
        if let Some(workspace_name) = name
            && workspace_name != "."
        {
            return Ok(workspace_name);
        }

        let cwd = env::current_dir()?;
        let worktrees = worktree::list(&self.project.path).await?;

        worktrees.into_iter().find(|wt| wt == &cwd).ok_or_else(|| {
            eyre::eyre!(
                "no workspace specified and not inside a worktree of project '{}'",
                self.project_name
            )
        })?;

        Ok(cwd
            .file_name()
            .ok_or_eyre("worktree path has no basename")?
            .to_string_lossy()
            .to_string())
    }
}

/// Check that the workspace is safe to tear down (clean git, no active execs).
pub(crate) fn safety_check(workspace: &Workspace, force: bool) -> eyre::Result<()> {
    if force {
        return Ok(());
    }

    if workspace.is_dirty() {
        eyre::bail!(
            "workspace '{}' has uncommitted changes (use --force to override)",
            workspace.name
        );
    }
    if !workspace.execs.is_empty() {
        eyre::bail!(
            "workspace '{}' has {} active exec session(s) (use --force to override)",
            workspace.name,
            workspace.execs.len()
        );
    }
    Ok(())
}

pub(crate) fn confirm() -> eyre::Result<bool> {
    eprint!("Proceed? [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

impl Cli {
    pub async fn run(self) -> eyre::Result<()> {
        let config = Config::load()?;
        let (project_name, project) = config.project(self.project)?;

        let state = State {
            docker: DockerClient::new().await?,
            project_name,
            project,
        };

        match self.command {
            Commands::Work(work) => work.run(state).await,
            Commands::Exec(exec) => exec.run(state).await,
            Commands::Fwd(fwd) => fwd.run(state).await,
            Commands::List(list) => list.run(state).await,
            Commands::Compose(compose) => compose.run(state).await,
            // Commands::Copy(copy) => copy.run(state).await,
            Commands::Archive(archive) => archive.run(state).await,
            Commands::Show(show) => show.run(state).await,
            Commands::Destroy(destroy) => destroy.run(state).await,
            Commands::Go(go) => go.run(state).await,
        }
    }
}
