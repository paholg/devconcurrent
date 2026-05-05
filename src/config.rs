use std::path::{Path, PathBuf};

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
        Self::load_from_path(&path)
    }

    pub(crate) fn load_from_path(path: &Path) -> eyre::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to load {}", path.display()))?;
        let de = toml::Deserializer::parse(&contents)
            .wrap_err_with(|| format!("failed to parse {}", path.display()))?;
        serde_path_to_error::deserialize(de)
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn project_order_is_stable() {
        let names = [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet", "kilo", "lima",
        ];
        let mut toml = String::new();
        for name in names {
            toml.push_str(&format!("[projects.{name}]\npath = \"/tmp/{name}\"\n\n"));
        }

        let mut file = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        file.write_all(toml.as_bytes()).unwrap();

        let first = Config::load_from_path(file.path()).unwrap();
        let expected: Vec<&str> = first.projects.keys().map(String::as_str).collect();
        assert_eq!(expected, names);

        for i in 0..50 {
            let cfg = Config::load_from_path(file.path()).unwrap();
            let got: Vec<&str> = cfg.projects.keys().map(String::as_str).collect();
            assert_eq!(got, expected, "project order changed on iteration {i}");
        }
    }
}
