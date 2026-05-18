use std::fmt;
use std::path::Path;

use owo_colors::OwoColorize;

#[derive(Debug, Default)]
pub(crate) struct GitStatus {
    pub(crate) ahead: usize,
    pub(crate) behind: usize,
    pub(crate) staged: usize,
    pub(crate) modified: usize,
    pub(crate) deleted: usize,
    pub(crate) untracked: usize,
    pub(crate) conflicted: usize,
    pub(crate) renamed: usize,
}

impl GitStatus {
    pub(crate) async fn fetch(path: &Path) -> eyre::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || fetch_sync(&path)).await?
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.staged + self.modified + self.deleted + self.untracked + self.conflicted + self.renamed
            > 0
    }
}

fn fetch_sync(path: &Path) -> eyre::Result<GitStatus> {
    let repo = gix::open(path)?;
    let mut gs = GitStatus::default();

    let (ahead, behind) = ahead_behind(&repo).unwrap_or((0, 0));
    gs.ahead = ahead;
    gs.behind = behind;

    // Use `git status` instead of gix's status API — the latter doesn't refresh
    // the index stat cache and reports false modifications in worktrees.
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-z"])
        .current_dir(path)
        .output()?;

    if !output.status.success() {
        return Ok(gs);
    }

    for entry in output.stdout.split(|&b| b == 0) {
        if entry.len() < 3 {
            continue;
        }
        let x = entry[0];
        let y = entry[1];
        match (x, y) {
            (b'?', b'?') => gs.untracked += 1,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => gs.conflicted += 1,
            _ => {
                match x {
                    b'A' | b'M' | b'T' | b'C' => gs.staged += 1,
                    b'R' => {
                        gs.staged += 1;
                        gs.renamed += 1;
                    }
                    b'D' => {
                        gs.staged += 1;
                        gs.deleted += 1;
                    }
                    _ => {}
                }
                match y {
                    b'M' | b'T' => gs.modified += 1,
                    b'D' => gs.deleted += 1,
                    _ => {}
                }
            }
        }
    }

    Ok(gs)
}

fn ahead_behind(repo: &gix::Repository) -> eyre::Result<(usize, usize)> {
    let head = repo.head()?;
    let head_id = head
        .id()
        .ok_or_else(|| eyre::eyre!("unborn HEAD"))?
        .detach();

    let referent = head
        .try_into_referent()
        .ok_or_else(|| eyre::eyre!("detached HEAD"))?;
    let tracking_ref_name = referent
        .remote_tracking_ref_name(gix::remote::Direction::Fetch)
        .ok_or_else(|| eyre::eyre!("no tracking branch"))??;
    let tracking_id = repo
        .find_reference(tracking_ref_name.as_ref())?
        .id()
        .detach();

    if head_id == tracking_id {
        return Ok((0, 0));
    }

    let ahead = repo
        .rev_walk([head_id])
        .with_hidden([tracking_id])
        .all()?
        .filter_map(Result::ok)
        .count();

    let behind = repo
        .rev_walk([tracking_id])
        .with_hidden([head_id])
        .all()?
        .filter_map(Result::ok)
        .count();

    Ok((ahead, behind))
}

impl fmt::Display for GitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();

        if self.ahead > 0 {
            s.push('⇡');
        }
        if self.behind > 0 {
            s.push('⇣');
        }
        if self.staged > 0 {
            s.push('+');
        }
        if self.modified > 0 {
            s.push('!');
        }
        if self.deleted > 0 {
            s.push('✘');
        }
        if self.renamed > 0 {
            s.push('»');
        }
        if self.untracked > 0 {
            s.push('?');
        }
        if self.conflicted > 0 {
            s.push('=');
        }

        write!(f, "{}", s.red())
    }
}
