//! Minimal client for the Docker / Podman Engine API.
//!
//! Talks to a local Unix socket via the versioned HTTP API. Pinned to API
//! v1.44 with negotiation: on connect we read `/version` from the daemon and
//! pick `min(OUR_MAX, daemon.ApiVersion)`. Both Docker and Podman's
//! Docker-compat endpoint are supported.

mod client;
mod container;
mod error;
mod request_ext;
mod socket;
mod types;

pub use client::Docker;
pub use container::{
    ContainerConfig, ContainerDetails, ContainerState, ContainerStatus, EndpointSettings,
    NetworkSettings,
};
pub use error::{Error, Result};
pub use socket::discover_socket;
pub use types::ApiVersion;
