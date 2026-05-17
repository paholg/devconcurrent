use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};

use crate::client::Docker;
use crate::error::Result;
use crate::request_ext::ReqwestExt;

/// Treat a JSON `null` as the type's `Default`. Docker uses `null` for empty
/// collections in some places (e.g. `"ExecIDs": null`), and bare `default`
/// only handles missing fields.
fn null_as_default<'de, T, D>(d: D) -> std::result::Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Option::unwrap_or_default)
}

/// Result of `GET /containers/{id}/json` — i.e. `docker inspect`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerDetails {
    pub id: String,
    pub created: String,
    pub state: ContainerState,
    pub config: ContainerConfig,
    pub network_settings: NetworkSettings,
    #[serde(rename = "ExecIDs", default, deserialize_with = "null_as_default")]
    pub exec_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerState {
    pub status: ContainerStatus,
    pub running: bool,
    pub exit_code: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerStatus {
    Created,
    Running,
    Paused,
    Restarting,
    Removing,
    Exited,
    Dead,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerConfig {
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub labels: IndexMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NetworkSettings {
    #[serde(default)]
    pub networks: IndexMap<String, EndpointSettings>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EndpointSettings {
    #[serde(rename = "IPAddress")]
    pub ip_address: Option<String>,
}

impl Docker {
    /// `GET /containers/{id}/json` — inspect a container.
    ///
    /// Returns [`Error::NotFound`] if the container doesn't exist (so callers
    /// can `match` on it).
    pub async fn inspect_container(&self, id: &str) -> Result<ContainerDetails> {
        let url = self.url(&format!("containers/{id}/json"));
        self.http().get(url).try_send().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_minimal_inspect_response() {
        // Trimmed real response shape; ensures permissive deserialization works
        // and that PascalCase + `ExecIDs` are both handled.
        let json = r#"{
            "Id": "abc123",
            "Created": "2024-01-15T12:00:00Z",
            "State": {
                "Status": "running",
                "Running": true,
                "ExitCode": 0
            },
            "Config": {
                "Env": ["PATH=/usr/bin", "FOO=bar"],
                "Labels": {"com.docker.compose.service": "app"}
            },
            "NetworkSettings": {
                "Networks": {
                    "bridge": {"IPAddress": "172.17.0.2"}
                }
            },
            "ExecIDs": ["e1", "e2"],
            "UnknownField": 42
        }"#;
        let details: ContainerDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.id, "abc123");
        assert_eq!(details.state.status, ContainerStatus::Running);
        assert!(details.state.running);
        assert_eq!(details.config.env.len(), 2);
        assert_eq!(
            details.network_settings.networks["bridge"]
                .ip_address
                .as_deref(),
            Some("172.17.0.2")
        );
        assert_eq!(details.exec_ids, vec!["e1", "e2"]);
    }

    #[test]
    fn null_exec_ids_becomes_empty() {
        // Docker returns `"ExecIDs": null` for containers with no execs.
        let json = r#"{
            "Id": "abc",
            "Created": "2024-01-15T12:00:00Z",
            "State": {"Status": "exited", "Running": false, "ExitCode": 0},
            "Config": {},
            "NetworkSettings": {},
            "ExecIDs": null
        }"#;
        let details: ContainerDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.exec_ids, Vec::<String>::new());
    }
}
