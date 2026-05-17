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

impl ContainerConfig {
    /// Parse [`Self::env`] entries (`"KEY=VALUE"`) into a map. Entries missing
    /// `=` are skipped.
    pub fn parsed_env(&self) -> IndexMap<String, String> {
        self.env
            .iter()
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                Some((key.to_string(), value.to_string()))
            })
            .collect()
    }
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
