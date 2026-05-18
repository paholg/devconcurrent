use std::collections::HashMap;

use bytes::Bytes;
use futures_util::stream::{Stream, StreamExt};
use indexmap::IndexMap;
use serde::Deserialize;
use snafu::ResultExt;

use crate::client::Docker;
use crate::error::{ApiSnafu, Error, JsonSnafu, Result};

/// One event from the Docker `/events` stream.
///
/// All fields are optional because the daemon emits a wide variety of event
/// shapes and we keep this type permissive.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EventMessage {
    /// Object kind the event applies to — `container`, `image`, `volume`, ….
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    /// What happened — `start`, `die`, `kill`, `destroy`, `health_status`, ….
    pub action: Option<String>,
    pub actor: EventActor,
    pub time: Option<i64>,
    #[serde(rename = "timeNano")]
    pub time_nano: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EventActor {
    #[serde(rename = "ID")]
    pub id: String,
    /// Includes labels copied onto the event by the daemon (e.g. `com.docker.compose.service`).
    #[serde(default)]
    pub attributes: IndexMap<String, String>,
}

/// Builder for [`Docker::events`].
pub struct EventsBuilder<'a> {
    docker: &'a Docker,
    filters: HashMap<&'static str, Vec<String>>,
}

impl Docker {
    /// `GET /events` — subscribe to the daemon's event stream.
    ///
    /// Returns a `Stream` of [`EventMessage`] values. Filters narrow the stream
    /// before the daemon sends bytes.
    pub fn events(&self) -> EventsBuilder<'_> {
        EventsBuilder {
            docker: self,
            filters: HashMap::new(),
        }
    }
}

impl<'a> EventsBuilder<'a> {
    pub fn with_label(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.filters.entry("label").or_default().push(format!(
            "{}={}",
            key.as_ref(),
            value.as_ref()
        ));
        self
    }

    pub fn with_label_key(mut self, key: impl Into<String>) -> Self {
        self.filters.entry("label").or_default().push(key.into());
        self
    }

    /// Filter on the actor object type (`container`, `image`, `volume`, ...).
    pub fn with_type(mut self, kind: impl Into<String>) -> Self {
        self.filters.entry("type").or_default().push(kind.into());
        self
    }

    /// Filter on the event action (`start`, `die`, ...).
    pub fn with_event(mut self, action: impl Into<String>) -> Self {
        self.filters.entry("event").or_default().push(action.into());
        self
    }

    /// Open the stream. The returned `Stream` yields one [`EventMessage`] per
    /// daemon event until the daemon closes the connection.
    pub async fn call(self) -> Result<impl Stream<Item = Result<EventMessage>> + 'static> {
        let mut url = self.docker.url("events");
        if !self.filters.is_empty() {
            let json = serde_json::to_string(&self.filters).expect("string-keyed map serializes");
            url.query_pairs_mut().append_pair("filters", &json);
        }
        let response = self.docker.http().get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return ApiSnafu {
                status: status.as_u16(),
                message,
            }
            .fail();
        }
        let bytes = response.bytes_stream().map(|r| r.map_err(Error::from));
        Ok(ndjson_lines(bytes))
    }
}

fn ndjson_lines<S>(stream: S) -> impl Stream<Item = Result<EventMessage>>
where
    S: Stream<Item = Result<Bytes>> + Unpin + 'static,
{
    futures_util::stream::try_unfold(
        (stream, Vec::<u8>::new()),
        |(mut stream, mut buf)| async move {
            loop {
                if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    let trimmed = trim_eol(&line);
                    if trimmed.is_empty() {
                        continue;
                    }
                    let event: EventMessage =
                        serde_json::from_slice(trimmed).context(JsonSnafu {
                            body: String::from_utf8_lossy(trimmed).into_owned(),
                        })?;
                    return Ok(Some((event, (stream, buf))));
                }
                match stream.next().await {
                    Some(Ok(chunk)) => buf.extend_from_slice(&chunk),
                    Some(Err(e)) => return Err(e),
                    None => {
                        let trimmed = trim_eol(&buf);
                        if trimmed.is_empty() {
                            return Ok(None);
                        }
                        let event: EventMessage =
                            serde_json::from_slice(trimmed).context(JsonSnafu {
                                body: String::from_utf8_lossy(trimmed).into_owned(),
                            })?;
                        return Ok(Some((event, (stream, Vec::new()))));
                    }
                }
            }
        },
    )
}

fn trim_eol(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && matches!(line[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    &line[..end]
}
