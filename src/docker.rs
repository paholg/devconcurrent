use std::collections::HashMap;

use bollard::{
    Docker,
    plugin::ContainerSummaryStateEnum,
    query_parameters::{ListContainersOptions, StatsOptions},
};
use derive_more::{Add, Sum};
use eyre::{WrapErr, eyre};
use futures::{StreamExt, future::try_join_all};

use crate::workspace::Workspace;

pub(crate) mod compose;

#[derive(Debug)]
pub(crate) struct ContainerInfo {
    pub(crate) id: String,
    pub(crate) state: ContainerSummaryStateEnum,
    pub(crate) dc_project: Option<String>,
    pub(crate) created: Option<i64>,
    pub(crate) host_ports: Vec<u16>,
}

#[derive(Debug, Clone, Default, Add, Sum)]
pub(crate) struct Stats {
    /// Current memory use in bytes.
    pub(crate) ram: u64,
}

pub(crate) struct DockerClient {
    // TODO: Instead of making this public, we should move all docker functionality we need to this
    // module.
    pub(crate) docker: Docker,
}

impl DockerClient {
    pub(crate) async fn new() -> eyre::Result<Self> {
        let docker =
            Docker::connect_with_local_defaults().wrap_err("failed to connect to Docker")?;
        Ok(Self { docker })
    }

    /// Return containers for a specific workspace, filtered at the Docker API level.
    pub(crate) async fn workspace_container_info(
        &self,
        workspace: &Workspace<'_>,
    ) -> eyre::Result<Vec<ContainerInfo>> {
        let label_filter = format!("devcontainer.local_folder={}", workspace.path.display());
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(HashMap::from([("label".to_string(), vec![label_filter])])),
                ..Default::default()
            }))
            .await?;

        let mut result = Vec::new();
        for c in containers {
            let dc_project = c
                .labels
                .as_ref()
                .and_then(|l| l.get("dev.devconcurrent.project").cloned());
            let id = c.id.ok_or_else(|| eyre!("container missing id"))?;
            let state = c.state.ok_or_else(|| eyre!("container missing state"))?;

            let host_ports: Vec<u16> = c
                .ports
                .unwrap_or_default()
                .iter()
                .filter_map(|p| p.public_port)
                .collect();

            result.push(ContainerInfo {
                id,
                state,
                dc_project,
                created: c.created,
                host_ports,
            });
        }

        Ok(result)
    }

    pub(crate) async fn stats(&self, container_id: &str) -> eyre::Result<Stats> {
        let mut stream = self.docker.stats(
            container_id,
            Some(StatsOptions {
                stream: false,
                one_shot: true,
            }),
        );
        match stream.next().await {
            Some(Ok(stats)) => {
                let ram = stats
                    .memory_stats
                    .as_ref()
                    .and_then(|m| m.usage)
                    .unwrap_or_default();
                Ok(Stats { ram })
            }
            Some(Err(e)) => Err(e.into()),
            None => Err(eyre!("no stats response for container {container_id}")),
        }
    }

    /// Ports forwarded by `dc fwd`.
    pub(crate) async fn forwarded_ports(
        &self,
        project: &str,
    ) -> eyre::Result<HashMap<String, Vec<u16>>> {
        let mut filters = HashMap::new();
        filters.insert(
            "label".into(),
            vec![
                "dev.devconcurrent.fwd=true".to_string(),
                format!("dev.devconcurrent.project={project}"),
            ],
        );
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: false,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;

        let result = containers
            .into_iter()
            .filter_map(|c| {
                let ws = c.labels?.get("dev.devconcurrent.workspace")?.clone();
                let ports: Vec<u16> = c.ports?.into_iter().filter_map(|p| p.public_port).collect();
                if ports.is_empty() {
                    None
                } else {
                    Some((ws, ports))
                }
            })
            .fold(HashMap::new(), |mut acc, (ws, ports)| {
                acc.entry(ws).or_insert_with(Vec::new).extend(ports);
                acc
            });
        Ok(result)
    }

    pub(crate) async fn is_forwarding_healthy(
        &self,
        workspace: &Workspace<'_>,
    ) -> eyre::Result<bool> {
        let mut labels = workspace.docker_labels();
        labels.push("dev.devconcurrent.fwd=true".to_string());
        let filters = HashMap::from([("label".into(), labels)]);

        let sidecars = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;

        let target_id = sidecars.iter().find_map(|c| {
            c.labels
                .as_ref()?
                .get("dev.devconcurrent.fwd.target")
                .cloned()
        });

        let Some(target_id) = target_id else {
            return Ok(sidecars.is_empty());
        };

        let mut filters = HashMap::new();
        filters.insert("id".into(), vec![target_id]);
        filters.insert("status".into(), vec!["running".to_string()]);
        let targets = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: false,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;
        Ok(!targets.is_empty())
    }

    pub(crate) async fn workspace_forwarded_ports(
        &self,
        workspace: &Workspace<'_>,
    ) -> eyre::Result<Vec<u16>> {
        let mut labels = workspace.docker_labels();
        labels.push("dev.devconcurrent.fwd=true".to_string());
        let filters = HashMap::from([("label".into(), labels)]);

        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: false,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;

        let mut ports: Vec<u16> = containers
            .into_iter()
            .flat_map(|c| {
                c.ports
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|p| p.public_port)
            })
            .collect();
        ports.sort_unstable();
        ports.dedup();
        Ok(ports)
    }

    /// Return (compose_service, ip_address) for every compose container in this workspace's
    /// project. Containers without a service label or without an IP are omitted.
    pub(crate) async fn workspace_compose_ips(
        &self,
        workspace: &Workspace<'_>,
    ) -> eyre::Result<Vec<(String, String)>> {
        let label_filter = format!(
            "com.docker.compose.project={}",
            workspace.compose_project_name()
        );
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(HashMap::from([("label".to_string(), vec![label_filter])])),
                ..Default::default()
            }))
            .await?;

        let mut result = Vec::new();
        for c in containers {
            let service = c
                .labels
                .as_ref()
                .and_then(|l| l.get("com.docker.compose.service").cloned());
            let ip = c
                .network_settings
                .as_ref()
                .and_then(|ns| ns.networks.as_ref())
                .and_then(|nets| {
                    nets.values()
                        .filter_map(|ep| ep.ip_address.as_deref())
                        .find(|ip| !ip.is_empty())
                        .map(str::to_string)
                });
            if let (Some(service), Some(ip)) = (service, ip) {
                result.push((service, ip));
            }
        }
        result.sort();
        Ok(result)
    }

    pub(crate) async fn execs(&self, container_id: &str) -> eyre::Result<usize> {
        let info = self
            .docker
            .inspect_container(container_id, None)
            .await
            .wrap_err_with(|| format!("failed to inspect container {container_id}"))?;
        let exec_ids = info.exec_ids.unwrap_or_default();

        let futures = exec_ids.into_iter().map(async |eid| -> eyre::Result<bool> {
            let running = self
                .docker
                .inspect_exec(&eid)
                .await?
                .running
                .unwrap_or(false);

            Ok(running)
        });

        let execs = try_join_all(futures)
            .await?
            .into_iter()
            .filter(|r| *r)
            .count();
        Ok(execs)
    }
}
