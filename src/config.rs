use std::path::PathBuf;

use eyre::{WrapErr, eyre};
use indexmap::IndexMap;
use serde::Deserialize;

use crate::helpers::{deserialize_shell_path, deserialize_shell_path_opt};

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) projects: IndexMap<String, Project>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Project {
    #[serde(deserialize_with = "deserialize_shell_path")]
    pub(crate) path: PathBuf,
    #[serde(default)]
    pub(crate) environment: IndexMap<String, String>,
    #[serde(default)]
    pub(crate) volumes: Vec<String>,
    #[serde(default)]
    pub(crate) exec: Exec,
    #[serde(default, deserialize_with = "deserialize_shell_path_opt")]
    pub(crate) worktree_folder: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct Exec {
    pub(crate) environment: IndexMap<String, String>,
}

impl Config {
    pub(crate) fn load() -> eyre::Result<Self> {
        let dirs = directories::ProjectDirs::from("", "", "devconcurrent")
            .ok_or_else(|| eyre::eyre!("could not determine config directory"))?;
        let path = dirs.config_dir().join("config.toml");
        let cfg = config::Config::builder()
            .add_source(config::File::from(path.clone()))
            .build()
            .wrap_err_with(|| format!("failed to load {}", path.display()))?;
        serde_path_to_error::deserialize(cfg)
            .wrap_err_with(|| format!("failed to parse {}", path.display()))
    }

    pub(crate) fn project(
        mut self,
        project_name: Option<String>,
    ) -> eyre::Result<(String, Project)> {
        let name = project_name.or_else(|| std::env::var("DC_PROJECT").ok());
        match name {
            Some(name) => self
                .projects
                .swap_remove_entry(&name)
                .ok_or_else(|| eyre!("no project configured with name: {name:?}")),
            None => self
                .projects
                .into_iter()
                .next()
                .ok_or_else(|| eyre!("no projects configured")),
        }
    }
}
