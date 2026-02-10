use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

pub async fn create(repo_path: &Path, workspace_dir: &Path, name: &str) -> eyre::Result<PathBuf> {
    // Validate it's a git repo
    gix::open(repo_path)?;

    let worktree_path = workspace_dir.join(name);
    if worktree_path.exists() {
        return Ok(worktree_path);
    }

    let status = Command::new("git")
        .args(["worktree", "add"])
        .arg(&worktree_path)
        .current_dir(repo_path)
        .stdout(Stdio::null())
        .status()
        .await?;
    eyre::ensure!(status.success(), "git worktree add failed");

    Ok(worktree_path)
}

pub async fn list(repo_path: &Path) -> eyre::Result<HashSet<PathBuf>> {
    let out = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .await?;
    eyre::ensure!(out.status.success(), "git worktree list failed");
    let output = String::from_utf8(out.stdout)?;

    Ok(output
        .lines()
        .filter_map(|line| line.strip_prefix("worktree ").map(PathBuf::from))
        .collect())
}
