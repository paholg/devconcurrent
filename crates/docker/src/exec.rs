use serde::Deserialize;

use crate::client::Docker;
use crate::error::Result;
use crate::request_ext::ReqwestExt;

/// Result of `GET /exec/{id}/json` — i.e. `docker exec inspect`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExecDetails {
    #[serde(rename = "ID")]
    pub id: String,
    pub running: bool,
    /// Exit code; `None` while still running.
    pub exit_code: Option<i64>,
}

impl Docker {
    /// `GET /exec/{id}/json` — inspect an exec instance.
    ///
    /// Returns [`crate::Error::NotFound`] if the exec doesn't exist.
    pub async fn inspect_exec(&self, id: &str) -> Result<ExecDetails> {
        let url = self.url(&format!("exec/{id}/json"));
        self.http().get(url).try_send().await
    }
}
