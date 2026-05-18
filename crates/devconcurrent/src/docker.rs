use std::collections::HashMap;

use bollard::{Docker, query_parameters::StatsOptions};
use derive_more::{Add, Sum};
use eyre::{WrapErr, eyre};
use futures::{StreamExt, future::try_join_all};

use crate::workspace::Workspace;

pub(crate) mod compose;
pub(crate) mod probe;

pub(crate) const LOCAL_FOLDER_LABEL: &str = "devcontainer.local_folder";

pub(crate) const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";
pub(crate) const COMPOSE_SERVICE_LABEL: &str = "com.docker.compose.service";

pub(crate) const MANAGED_LABEL: &str = "dev.devconcurrent.managed";
pub(crate) const PROJECT_LABEL: &str = "dev.devconcurrent.project";
pub(crate) const WORKSPACE_LABEL: &str = "dev.devconcurrent.workspace";
pub(crate) const FORWARD_LABEL: &str = "dev.devconcurrent.fwd";
pub(crate) const FORWARD_TARGET_LABEL: &str = "dev.devconcurrent.fwd.target";

#[derive(Debug)]
pub(crate) struct ContainerInfo {
    pub(crate) id: String,
    pub(crate) state: docker::ContainerStatus,
    pub(crate) dc_project: Option<String>,
    pub(crate) created: i64,
    pub(crate) host_ports: Vec<u16>,
}

#[derive(Debug, Clone, Default, Add, Sum)]
pub(crate) struct Stats {
    /// Current memory use in bytes.
    pub(crate) ram: u64,
}

fn container_info_from(c: docker::ContainerSummary) -> ContainerInfo {
    let dc_project = c.labels.get(PROJECT_LABEL).cloned();
    let host_ports: Vec<u16> = c.ports.iter().filter_map(|p| p.public_port).collect();
    ContainerInfo {
        id: c.id,
        state: c.state,
        dc_project,
        created: c.created,
        host_ports,
    }
}

pub(crate) struct DockerClient {
    // TODO: Instead of making this public, we should move all docker functionality we need to this
    // module.
    pub(crate) docker: Docker,
    pub(crate) client: docker::Docker,
}

impl DockerClient {
    pub(crate) async fn new() -> eyre::Result<Self> {
        let docker =
            Docker::connect_with_local_defaults().wrap_err("failed to connect to Docker")?;
        let client = docker::Docker::connect()
            .await
            .wrap_err("failed to connect via docker crate")?;
        Ok(Self { docker, client })
    }

    /// Return containers for a specific workspace, filtered at the Docker API level.
    pub(crate) async fn workspace_container_info(
        &self,
        workspace: &Workspace<'_>,
    ) -> eyre::Result<Vec<ContainerInfo>> {
        let summaries = self
            .client
            .list_containers()
            .all(true)
            .with_label(LOCAL_FOLDER_LABEL, workspace.path.display().to_string())
            .call()
            .await?;
        Ok(summaries.into_iter().map(container_info_from).collect())
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
        let summaries = self
            .client
            .list_containers()
            .with_label(FORWARD_LABEL, "true")
            .with_label(PROJECT_LABEL, project)
            .call()
            .await?;

        let result = summaries
            .into_iter()
            .filter_map(|c| {
                let ws = c.labels.get(WORKSPACE_LABEL)?.clone();
                let ports: Vec<u16> = c.ports.into_iter().filter_map(|p| p.public_port).collect();
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
        let sidecars = self
            .client
            .list_containers()
            .all(true)
            .with_label(PROJECT_LABEL, workspace.state.project_name.as_str())
            .with_label(WORKSPACE_LABEL, workspace.name.as_str())
            .with_label(FORWARD_LABEL, "true")
            .call()
            .await?;

        let target_id = sidecars
            .iter()
            .find_map(|c| c.labels.get(FORWARD_TARGET_LABEL).cloned());

        let Some(target_id) = target_id else {
            return Ok(sidecars.is_empty());
        };

        let targets = self
            .client
            .list_containers()
            .with_id(target_id)
            .with_status(docker::ContainerStatus::Running)
            .call()
            .await?;
        Ok(!targets.is_empty())
    }

    pub(crate) async fn workspace_forwarded_ports(
        &self,
        workspace: &Workspace<'_>,
    ) -> eyre::Result<Vec<u16>> {
        let summaries = self
            .client
            .list_containers()
            .with_label(PROJECT_LABEL, workspace.state.project_name.as_str())
            .with_label(WORKSPACE_LABEL, workspace.name.as_str())
            .with_label(FORWARD_LABEL, "true")
            .call()
            .await?;

        let mut ports: Vec<u16> = summaries
            .into_iter()
            .flat_map(|c| c.ports.into_iter().filter_map(|p| p.public_port))
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
        let summaries = self
            .client
            .list_containers()
            .all(true)
            .with_label(COMPOSE_PROJECT_LABEL, workspace.compose_project_name())
            .call()
            .await?;

        let mut result = Vec::new();
        for c in summaries {
            let service = c.labels.get(COMPOSE_SERVICE_LABEL).cloned();
            let ip = c
                .network_settings
                .networks
                .values()
                .filter_map(|ep| ep.ip_address.as_deref())
                .find(|ip| !ip.is_empty())
                .map(str::to_string);
            if let (Some(service), Some(ip)) = (service, ip) {
                result.push((service, ip));
            }
        }
        result.sort();
        Ok(result)
    }

    pub(crate) async fn execs(&self, container_id: &str) -> eyre::Result<usize> {
        let info = self
            .client
            .inspect_container(container_id)
            .await
            .wrap_err_with(|| format!("failed to inspect container {container_id}"))?;

        let futures = info
            .exec_ids
            .into_iter()
            .map(async |eid| -> eyre::Result<bool> {
                Ok(self.client.inspect_exec(&eid).await?.running)
            });

        let execs = try_join_all(futures)
            .await?
            .into_iter()
            .filter(|r| *r)
            .count();
        Ok(execs)
    }
}
