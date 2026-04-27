use std::io::{BufRead, Write};

use clap::{Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;

use crate::{complete, state::State, workspace::Workspace};

mod compose;
mod destroy;
mod exec;
pub(crate) mod fwd;
mod go;
mod list;
mod show;
mod up;

const ABOUT: &str =
    "A tool for managing devcontainers, especially when combined with git worktrees";

#[derive(Debug, Parser)]
#[command(version, about = ABOUT)]
pub(crate) struct Cli {
    #[arg(
        short,
        long,
        help = "name of project [default: The DC_PROJECT variable, then the first configured project]",
        add = ArgValueCompleter::new(complete::complete_project),
    )]
    pub(crate) project: Option<String>,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    #[command()]
    Up(up::Up),
    #[command(visible_alias = "x")]
    Exec(exec::Exec),
    #[command(visible_alias = "f")]
    Fwd(fwd::Fwd),
    #[command(visible_alias = "l")]
    List(list::List),
    #[command(visible_alias = "c")]
    Compose(compose::Compose),
    #[command()]
    Destroy(destroy::Destroy),
    Show(show::Show),
    #[command()]
    Go(go::Go),
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
    if workspace.execs > 0 {
        eyre::bail!(
            "workspace '{}' has {} active exec session(s) (use --force to override)",
            workspace.name,
            workspace.execs
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
    pub(crate) async fn run(self) -> eyre::Result<()> {
        let state = State::new(self.project).await?;

        match self.command {
            Commands::Up(up) => up.run(state).await,
            Commands::Exec(exec) => exec.run(state).await,
            Commands::Fwd(fwd) => fwd.run(state).await,
            Commands::List(list) => list.run(state).await,
            Commands::Compose(compose) => compose.run(state).await,
            // Commands::Copy(copy) => copy.run(state).await,
            Commands::Show(show) => show.run(state).await,
            Commands::Destroy(destroy) => destroy.run(state).await,
            Commands::Go(go) => go.run(state).await,
        }
    }
}
