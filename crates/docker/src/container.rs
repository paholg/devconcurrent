use std::fmt;
use std::net::IpAddr;

use bon::bon;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};

use crate::client::Docker;
use crate::error::Result;
use crate::filter::{Filter, FilterSliceExt};
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

impl fmt::Display for ContainerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Restarting => "restarting",
            Self::Removing => "removing",
            Self::Exited => "exited",
            Self::Dead => "dead",
        })
    }
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerSummary {
    pub id: String,
    #[serde(default)]
    pub names: Vec<String>,
    pub image: String,
    pub state: ContainerStatus,
    pub created: i64,
    #[serde(default)]
    pub labels: IndexMap<String, String>,
    #[serde(default)]
    pub ports: Vec<Port>,
    #[serde(default)]
    pub network_settings: NetworkSettings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Port {
    #[serde(rename = "IP")]
    pub ip: Option<IpAddr>,
    pub private_port: u16,
    pub public_port: Option<u16>,
    #[serde(rename = "Type")]
    pub kind: PortType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortType {
    Tcp,
    Udp,
    Sctp,
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

#[bon]
impl Docker {
    /// `GET /containers/json` — list containers, with optional filters.
    ///
    /// `all = false` (the default) returns only running containers, matching
    /// the `docker ps` default. Filters are added via [`.with_label()`],
    /// [`.with_status()`], etc. on the returned builder.
    ///
    /// [`.with_label()`]: DockerListContainersBuilder::with_label
    /// [`.with_status()`]: DockerListContainersBuilder::with_status
    #[builder]
    pub async fn list_containers(
        &self,
        #[builder(field)] filters: Vec<Filter>,
        #[builder(default)] all: bool,
    ) -> Result<Vec<ContainerSummary>> {
        let mut url = self.url("containers/json");
        {
            let mut pairs = url.query_pairs_mut();
            if all {
                pairs.append_pair("all", "true");
            }
            if !filters.is_empty() {
                pairs.append_pair("filters", &filters.to_docker_query());
            }
        }
        self.http().get(url).try_send().await
    }
}

impl<S: docker_list_containers_builder::State> DockerListContainersBuilder<'_, S> {
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters.push(Filter::Label {
            key: key.into(),
            value: Some(value.into()),
        });
        self
    }

    pub fn with_label_key(mut self, key: impl Into<String>) -> Self {
        self.filters.push(Filter::Label {
            key: key.into(),
            value: None,
        });
        self
    }

    pub fn with_status(mut self, status: ContainerStatus) -> Self {
        self.filters.push(Filter::Status(status));
        self
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.filters.push(Filter::Id(id.into()));
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.filters.push(Filter::Name(name.into()));
        self
    }
}
