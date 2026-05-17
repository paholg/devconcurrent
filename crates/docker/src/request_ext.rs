use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

use crate::Error;
use crate::error::ApiSnafu;

pub(crate) trait ReqwestExt {
    async fn try_send<T: DeserializeOwned>(self) -> crate::Result<T>;
}

impl ReqwestExt for RequestBuilder {
    async fn try_send<T: DeserializeOwned>(self) -> crate::Result<T> {
        let response = self.send().await?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Err(Error::NotFound);
        }
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return ApiSnafu {
                status: status.as_u16(),
                message,
            }
            .fail();
        }
        let body = response.bytes().await?;

        match serde_json::from_slice(&body) {
            Ok(r) => Ok(r),
            Err(error) => {
                tracing::debug!(?error, ?body, "failed to parse response");
                Err(error.into())
            }
        }
    }
}
