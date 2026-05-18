use serde::Deserialize;

use crate::client::Docker;
use crate::error::Result;
use crate::request_ext::ReqwestExt;

/// One-shot stats snapshot for a container (`GET /containers/{id}/stats?stream=false&one-shot=true`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContainerStats {
    #[serde(default)]
    pub memory_stats: MemoryStats,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemoryStats {
    /// Current memory use in bytes. `None` if not available (e.g. cgroup v1
    /// without memory accounting enabled).
    pub usage: Option<u64>,
}

impl Docker {
    /// `GET /containers/{id}/stats?stream=false&one-shot=true` — a single
    /// snapshot of cgroup-level stats.
    pub async fn stats(&self, id: &str) -> Result<ContainerStats> {
        let mut url = self.url(&format!("containers/{id}/stats"));
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("stream", "false");
            pairs.append_pair("one-shot", "true");
        }
        self.http().get(url).try_send().await
    }
}
