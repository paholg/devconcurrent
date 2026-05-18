use serde::Deserialize;

use crate::client::Docker;
use crate::error::{ApiSnafu, Result};
use crate::request_ext::ReqwestExt;

/// Subset of `GET /images/{name}/json`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageDetails {
    pub id: String,
    #[serde(default)]
    pub repo_tags: Vec<String>,
}

/// One progress event in the NDJSON stream returned by `POST /images/create`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullEvent {
    status: Option<String>,
    error: Option<String>,
    error_detail: Option<ErrorDetail>,
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorDetail {
    message: String,
}

impl Docker {
    /// Pull the image if it isn't already present locally. No-op if it is.
    pub async fn ensure_image(&self, name: &str) -> Result<()> {
        match self.inspect_image(name).await {
            Ok(_) => Ok(()),
            Err(crate::Error::NotFound) => self.pull_image(name).await,
            Err(e) => Err(e),
        }
    }

    /// `GET /images/{name}/json` — inspect an image.
    ///
    /// Returns [`crate::Error::NotFound`] if the image isn't locally available.
    pub async fn inspect_image(&self, name: &str) -> Result<ImageDetails> {
        let url = self.url(&format!("images/{name}/json"));
        self.http().get(url).try_send().await
    }

    /// `POST /images/create?fromImage=<name>` — pull an image.
    ///
    /// Drains the daemon's NDJSON progress stream and only reports the final
    /// outcome; per-layer progress is dropped. If any line in the stream
    /// carries an error event, surface it as [`crate::Error::Api`].
    pub async fn pull_image(&self, name: &str) -> Result<()> {
        let mut url = self.url("images/create");
        url.query_pairs_mut().append_pair("fromImage", name);

        let events: Vec<PullEvent> = self.http().post(url).try_send_ndjson().await?;
        for event in events {
            if event.error.is_some() || event.error_detail.is_some() {
                let message = event
                    .error_detail
                    .map(|d| d.message)
                    .or(event.error)
                    .unwrap_or_else(|| event.status.unwrap_or_default());
                return ApiSnafu {
                    status: 0u16,
                    message,
                }
                .fail();
            }
        }
        Ok(())
    }
}
