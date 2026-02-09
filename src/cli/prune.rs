use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::ansi::{CYAN, GREEN, RED, RESET, YELLOW};
use crate::config::Config;
use crate::devcontainer::DevContainer;
use crate::runner::{self, Runnable};
use crate::workspace::{Speed, Workspace, workspace_table};
use bollard::Docker;
use clap::Args;
use tokio::process::Command;
use tracing::trace;

#[derive(Debug, Args)]
pub struct Prune {
    #[arg(
        short,
        long,
        help = "name of project [default: The first one configured]"
    )]
    project: Option<String>,

    #[arg(short, long, help = "skip confirmation prompt")]
    yes: bool,
}

impl Prune {
    pub async fn run(self, docker: &Docker, config: &Config) -> eyre::Result<()> {
        let (_, project) = config.project(self.project.as_deref())?;
        let dc = DevContainer::load(project)?;
        let dc_options = dc.common.customizations.dc;

        let worktrees = list_worktrees(&project.path, &dc_options.workspace_dir()).await?;
        if worktrees.is_empty() {
            trace!("Nothing to prune.");
            return Ok(());
        }

        let workspaces =
            Workspace::list_project(docker, self.project.as_deref(), config, Speed::Slow).await?;
        let ws_map: HashMap<&Path, &Workspace> = workspaces
            .iter()
            .map(|ws| (ws.path.as_path(), ws))
            .collect();

        let mut in_use = Vec::new();
        let mut dirty = Vec::new();
        let mut to_clean_ws = Vec::new();
        let mut to_clean_orphans = Vec::new();

        for path in worktrees {
            if !path.exists() {
                to_clean_orphans.push(path);
            } else if let Some(ws) = ws_map.get(path.as_path()) {
                if !ws.execs.is_empty() {
                    in_use.push(*ws);
                } else if ws.dirty {
                    dirty.push(*ws);
                } else {
                    to_clean_ws.push(*ws);
                }
            } else {
                to_clean_orphans.push(path);
            }
        }

        if !in_use.is_empty() {
            println!("{GREEN}In Use{RESET} ({CYAN}skipping{RESET}):");
            print!("{}", workspace_table(in_use.iter().copied())?);
            println!();
        }
        if !dirty.is_empty() {
            println!("{RED}Dirty{RESET} ({CYAN}skipping{RESET}):");
            print!("{}", workspace_table(dirty.iter().copied())?);
            println!();
        }

        if to_clean_ws.is_empty() && to_clean_orphans.is_empty() {
            return Ok(());
        }

        println!("{YELLOW}Will Remove - DATA WILL BE LOST{RESET}:");
        if !to_clean_ws.is_empty() {
            print!("{}", workspace_table(to_clean_ws.iter().copied())?);
        }
        for p in &to_clean_orphans {
            println!("  {}", p.display());
        }
        println!();

        if !self.yes && !confirm()? {
            println!("Aborted.");
            return Ok(());
        }

        let mut cleanups: Vec<Cleanup> = Vec::new();
        for ws in &to_clean_ws {
            cleanups.push(Cleanup {
                repo_path: &project.path,
                path: &ws.path,
                compose_name: super::up::compose_project_name(&ws.path),
                remove_worktree: true,
                force: false,
            });
        }
        for path in &to_clean_orphans {
            cleanups.push(Cleanup {
                repo_path: &project.path,
                path,
                compose_name: super::up::compose_project_name(path),
                remove_worktree: true,
                force: false,
            });
        }
        let cleanups = CleanupMany { cleanups };
        runner::run("", &cleanups, None).await?;

        Ok(())
    }
}

async fn list_worktrees(repo_path: &Path, workspace_dir: &Path) -> eyre::Result<Vec<PathBuf>> {
    let out = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .await?;
    eyre::ensure!(out.status.success(), "git worktree list failed");
    let output = String::from_utf8(out.stdout)?;

    let workspace_dir = workspace_dir.canonicalize()?;
    let mut worktrees = Vec::new();

    for line in output.lines() {
        if let Some(path_str) = line.strip_prefix("worktree ") {
            let path = PathBuf::from(path_str);
            if path.starts_with(&workspace_dir) {
                worktrees.push(path);
            }
        }
    }

    Ok(worktrees)
}

struct CleanupMany<'a> {
    cleanups: Vec<Cleanup<'a>>,
}

impl Runnable for CleanupMany<'_> {
    fn command(&self) -> Cow<'_, str> {
        let paths = self
            .cleanups
            .iter()
            .map(|c| c.path.display().to_string())
            .collect::<Vec<_>>();

        paths.join(", ").into()
    }

    async fn run(&self, _dir: Option<&Path>) -> eyre::Result<()> {
        let labeled: Vec<_> = self
            .cleanups
            .iter()
            .map(|c| (c.path.display().to_string().into(), c))
            .collect();
        crate::runner::run_parallel(labeled).await
    }
}

pub(super) struct Cleanup<'a> {
    pub(super) repo_path: &'a Path,
    pub(super) path: &'a Path,
    pub(super) compose_name: String,
    pub(super) remove_worktree: bool,
    pub(super) force: bool,
}

impl Runnable for Cleanup<'_> {
    fn command(&self) -> Cow<'_, str> {
        format!("prune {}", self.path.display()).into()
    }

    async fn run(&self, _dir: Option<&Path>) -> eyre::Result<()> {
        let down_result = Command::new("docker")
            .args([
                "compose",
                "-p",
                &self.compose_name,
                "down",
                "-v",
                "--remove-orphans",
            ])
            .status()
            .await;

        down_result?;

        let override_file =
            std::env::temp_dir().join(format!("{}-override.yml", self.compose_name));
        if override_file.exists() {
            std::fs::remove_file(&override_file)?;
        }

        if self.remove_worktree {
            let mut args = vec!["worktree", "remove"];
            if self.force {
                args.push("--force");
            }
            let path_str = self.path.to_string_lossy();
            args.push(&path_str);

            let status = Command::new("git")
                .args(&args)
                .current_dir(self.repo_path)
                .status()
                .await?;
            eyre::ensure!(status.success(), "git worktree remove failed");
        }

        println!("Removed {}", self.path.display());
        Ok(())
    }
}

pub(super) fn confirm() -> eyre::Result<bool> {
    print!("Proceed? [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}
