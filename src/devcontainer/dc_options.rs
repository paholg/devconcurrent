use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;
use serde_with::serde_as;

use crate::helpers::deserialize_shell_path_opt;
use crate::run::cmd::Cmd;

#[serde_as]
#[serde_inline_default]
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DcOptions {
    pub default_exec: Option<Cmd>,
    #[serde(default, deserialize_with = "deserialize_shell_path_opt")]
    pub worktree_folder: Option<PathBuf>,
    /// Whether to mount the project's git directory into each workspace's devcontainer.
    ///
    /// Git worktrees have a simple `.git` file that points to the actual `.git` directory. If that
    /// directory isn't available, then no git commands will work in the worktree. By mounting it
    /// at its original path in the devcontainer, we allow you to use `git` freely for the workspace,
    /// both inside and out of the devcontainer.
    #[serde_inline_default(true)]
    pub mount_git: bool,
}
