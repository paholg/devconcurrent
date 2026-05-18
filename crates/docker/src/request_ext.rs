use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

use crate::Error;
use crate::error::ApiSnafu;

pub(crate) trait ReqwestExt {
    async fn try_send<T: DeserializeOwned>(self) -> crate::Result<T>;
    async fn try_send_empty(self) -> crate::Result<()>;
}

impl ReqwestExt for RequestBuilder {
    async fn try_send<T: DeserializeOwned>(self) -> crate::Result<T> {
        let body = check_response_body(self).await?;
        match serde_json::from_slice(&body) {
            Ok(r) => Ok(r),
            Err(error) => {
                tracing::debug!(?error, ?body, "failed to parse response");
                Err(error.into())
            }
        }
    }

    async fn try_send_empty(self) -> crate::Result<()> {
        check_response_body(self).await.map(drop)
    }
}

/// Send and validate; returns the response body bytes on success. Maps 404 to
/// [`Error::NotFound`] and other non-success statuses to [`Error::Api`].
async fn check_response_body(request: RequestBuilder) -> crate::Result<bytes::Bytes> {
    let response = request.send().await?;
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
    Ok(response.bytes().await?)
}
