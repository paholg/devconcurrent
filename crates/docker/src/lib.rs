//! Minimal client for the Docker / Podman Engine API.
//!
//! Talks to a local Unix socket via the versioned HTTP API. Pinned to API
//! v1.44 with negotiation: on connect we read `/version` from the daemon and
//! pick `min(OUR_MAX, daemon.ApiVersion)`. Both Docker and Podman's
//! Docker-compat endpoint are supported.

mod archive;
mod client;
mod container;
mod error;
mod events;
mod exec;
mod filter;
mod images;
mod request_ext;
mod socket;
mod stats;
mod types;
mod volumes;

#[cfg(feature = "docker-tests")]
pub mod test_support;

pub use archive::{build_archive, build_single_file_tar};
pub use client::Docker;
pub use container::{
    ContainerConfig, ContainerDetails, ContainerState, ContainerStatus, ContainerSummary,
    EndpointSettings, NetworkSettings, Port, PortType,
};
pub use error::{Error, Result};
pub use events::{EventActor, EventMessage, EventsBuilder};
pub use exec::ExecDetails;
pub use filter::Filter;
pub use images::ImageDetails;
pub use socket::discover_socket;
pub use stats::{ContainerStats, MemoryStats};
pub use types::ApiVersion;
pub use volumes::Volume;
