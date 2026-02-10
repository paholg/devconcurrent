use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::run::cmd::Cmd;

fn deserialize_shell_path_opt<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<PathBuf>, D::Error> {
    Option::<String>::deserialize(d)
        .map(|o| o.map(|s| PathBuf::from(shellexpand::tilde(&s).as_ref())))
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DcOptions {
    pub default_exec: Option<Cmd>,
    #[serde(default, deserialize_with = "deserialize_shell_path_opt")]
    worktree_folder: Option<PathBuf>,
    /// If set, this port will be used automatically by the `dc fwd` command, to
    /// map a static host port to the container of your choice.
    pub forward_port: Option<u16>,
    /// Port inside the container to forward to. Defaults to `fwd_port` if unset.
    pub container_port: Option<u16>,
    pub default_copy_volumes: Option<Vec<String>>,
}

impl DcOptions {
    pub fn workspace_dir(&self) -> PathBuf {
        self.worktree_folder.clone().unwrap_or("/tmp/".into())
    }
}
