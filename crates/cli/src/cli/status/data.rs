use std::{collections::HashMap, fmt};

use docker::ContainerStatus;

use crate::{
    ansi::{BLUE, GREEN, RED, RESET, YELLOW},
    bytes::Bytes,
    table::{Datum, Gatherer},
};

/// Docker-derived data sources for one workspace (per-workspace view). Git is
/// gathered separately since it needs no Docker.
pub(crate) struct WsSources {
    pub info: Gatherer<Option<Info>>,
    pub stats: Gatherer<Option<Stats>>,
    pub execs: Gatherer<Datum<Execs>>,
}

/// One container row (per-container view).
pub(crate) struct ContainerRow {
    pub id: String,
    pub service: String,
    /// Container (private) ports it exposes; used to attribute forwarded ports.
    pub exposed: Vec<u16>,
}

/// Per-container data sources (per-container view).
pub(crate) struct ContainerSources {
    pub stats: Gatherer<Option<Stats>>,
    pub execs: Gatherer<Datum<Execs>>,
}

/// Live container states keyed by id, from one `list_containers` call.
pub(crate) type ContainerStates = HashMap<String, ContainerState>;

/// A container status, colored by liveness.
#[derive(Clone, Copy)]
pub(crate) struct ContainerState(pub ContainerStatus);

impl fmt::Display for ContainerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let color = match self.0 {
            ContainerStatus::Running => GREEN,
            ContainerStatus::Exited | ContainerStatus::Dead => RED,
            ContainerStatus::Created
            | ContainerStatus::Paused
            | ContainerStatus::Removing
            | ContainerStatus::Restarting
            | ContainerStatus::Stopping => YELLOW,
        };
        write!(f, "{color}{}{RESET}", self.0)
    }
}

/// A CPU percentage, colored by load.
#[derive(Clone, Copy)]
pub(crate) struct Cpu(pub f64);

impl fmt::Display for Cpu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let color = if self.0 < 50.0 {
            GREEN
        } else if self.0 < 100.0 {
            YELLOW
        } else {
            RED
        };
        write!(f, "{color}{:.0}%{RESET}", self.0)
    }
}

/// A running-exec count; zero renders blank.
#[derive(Clone, Copy)]
pub(crate) struct Execs(pub usize);

impl fmt::Display for Execs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => Ok(()),
            n => write!(f, "{n}"),
        }
    }
}

/// Forwarded (`dc fwd`) ports.
pub(crate) struct Ports(pub Vec<u16>);

impl fmt::Display for Ports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, p) in self.0.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            write!(f, "{sep}{BLUE}{p}{RESET}")?;
        }
        Ok(())
    }
}

/// One `list_containers` call: status, docker ports, and the ids stats/execs
/// need. Same command, so gathered together.
pub(crate) struct Info {
    pub status: Datum<ContainerState>,
    pub ids: Vec<String>,
}

/// One round of `stats` calls. Mem and CPU share the command.
pub(crate) struct Stats {
    pub mem: Datum<Bytes>,
    pub cpu: Datum<Cpu>,
}

/// Previous CPU counters for one container, to diff against.
#[derive(Clone, Copy)]
pub(crate) struct PrevSample {
    pub total: u64,
    pub system: u64,
}

pub(crate) type FwdPorts = HashMap<String, Vec<u16>>;
