use std::path::PathBuf;

use eyre::eyre;
use indexmap::IndexMap;
use serde::Deserialize;

pub fn deserialize_shell_path<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
    let s = String::deserialize(d)?;
    Ok(PathBuf::from(shellexpand::tilde(&s).as_ref()))
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub projects: IndexMap<String, Project>,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    #[serde(deserialize_with = "deserialize_shell_path")]
    pub path: PathBuf,
}

impl Config {
    pub fn load() -> eyre::Result<Self> {
        let dirs = directories::ProjectDirs::from("", "", "dc")
            .ok_or_else(|| eyre::eyre!("could not determine config directory"))?;
        let path = dirs.config_dir().join("config.toml");
        let cfg = config::Config::builder()
            .add_source(config::File::from(path))
            .build()?;
        Ok(cfg.try_deserialize()?)
    }

    pub fn project(mut self, name: Option<&str>) -> eyre::Result<(String, Project)> {
        match name {
            Some(name) => self
                .projects
                .swap_remove_entry(name)
                .ok_or_else(|| eyre!("no project configured with name: {name}")),
            None => self
                .projects
                .into_iter()
                .next()
                .ok_or_else(|| eyre!("no projects configured")),
        }
    }
}
