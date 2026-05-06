use std::collections::BTreeMap;

use eyre::eyre;
use indexmap::IndexMap;
use num_bigint::BigUint;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub(crate) struct ContainerData {
    pub(crate) env: IndexMap<String, String>,
    pub(crate) labels: IndexMap<String, String>,
}

impl ContainerData {
    /// Read `Config.Env` and `Config.Labels` via `docker inspect`.
    pub(crate) async fn inspect(container_id: &str) -> eyre::Result<Self> {
        #[derive(Deserialize)]
        struct Raw {
            env: Option<Vec<String>>,
            labels: Option<IndexMap<String, String>>,
        }

        let output = tokio::process::Command::new("docker")
            .args([
                "inspect",
                "--format",
                r#"{"env":{{json .Config.Env}},"labels":{{json .Config.Labels}}}"#,
                container_id,
            ])
            .output()
            .await?;
        if !output.status.success() {
            return Err(eyre!(
                "docker inspect failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let raw: Raw = serde_json::from_slice(&output.stdout)?;
        Ok(Self {
            env: raw
                .env
                .unwrap_or_default()
                .into_iter()
                .filter_map(parse_env_pair)
                .collect(),
            labels: raw.labels.unwrap_or_default(),
        })
    }

    /// Compute `${devcontainerId}`: SHA-256 of the JSON-encoded `devcontainer.*` labels (with
    /// keys sorted), interpreted as a big-endian unsigned integer and base-32 encoded, padded
    /// to 52 chars. Mirrors [the reference impl][ref].
    ///
    /// [ref]: https://github.com/devcontainers/cli/blob/main/src/spec-common/variableSubstitution.ts
    pub(crate) fn devcontainer_id(&self) -> String {
        let id_labels: BTreeMap<&str, &str> = self
            .labels
            .iter()
            .filter(|(key, _)| key.starts_with("devcontainer."))
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let json = serde_json::to_string(&id_labels).expect("string-keyed map always serializes");
        let digest = Sha256::digest(json.as_bytes());
        format!("{:0>52}", BigUint::from_bytes_be(&digest).to_str_radix(32))
    }
}

fn parse_env_pair(pair: String) -> Option<(String, String)> {
    let (key, value) = pair.split_once('=')?;
    Some((key.to_string(), value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn devcontainer_id_format() {
        let data = ContainerData {
            env: IndexMap::new(),
            labels: map(&[
                ("devcontainer.local_folder", "/host/projects/myrepo"),
                (
                    "devcontainer.config_file",
                    "/host/projects/myrepo/.devcontainer/devcontainer.json",
                ),
                ("dev.devconcurrent.project", "myrepo"),
            ]),
        };
        let id = data.devcontainer_id();
        assert_eq!(id.len(), 52);
        assert!(
            id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='v')),
            "unexpected character in {id}",
        );
    }

    #[test]
    fn devcontainer_id_stable_across_label_order() {
        let a = ContainerData {
            env: IndexMap::new(),
            labels: map(&[
                ("devcontainer.local_folder", "/foo"),
                ("devcontainer.config_file", "/foo/.devcontainer.json"),
            ]),
        };
        let b = ContainerData {
            env: IndexMap::new(),
            labels: map(&[
                ("devcontainer.config_file", "/foo/.devcontainer.json"),
                ("devcontainer.local_folder", "/foo"),
            ]),
        };
        assert_eq!(a.devcontainer_id(), b.devcontainer_id());
    }

    #[test]
    fn devcontainer_id_ignores_non_id_labels() {
        let base = ContainerData {
            env: IndexMap::new(),
            labels: map(&[("devcontainer.local_folder", "/foo")]),
        };
        let with_extra = ContainerData {
            env: IndexMap::new(),
            labels: map(&[
                ("devcontainer.local_folder", "/foo"),
                ("dev.devconcurrent.project", "anything"),
            ]),
        };
        assert_eq!(base.devcontainer_id(), with_extra.devcontainer_id());
    }
}
