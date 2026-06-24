use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Args;

use crate::bytes::Bytes;
use crate::cli::status::data::{
    ContainerState, Cpu, Execs, FwdPorts, Info, Ports, PrevSample, Stats, WsSources,
};
use crate::config::Config;
use crate::docker::DockerClient;
use crate::state::State;
use crate::table::{Align, ColumnDef, Datum, Gatherer, TableBuilder, text, value};
use crate::workspace::Workspace;
use crate::workspace::git_status::GitStatus;

mod data;

const PERIOD: Duration = Duration::from_secs(1);

/// Show project or workspace status
#[derive(Debug, Args)]
pub(crate) struct Status {
    /// Show live, updating data
    #[arg(short, long)]
    live: bool,
}

/// A selectable status column. Builds its [`ColumnDef`] from the gathered
/// sources; the set of columns will eventually be user-configurable.
#[derive(Clone, Copy)]
pub(crate) enum Column {
    Name,
    Status,
    Mem,
    Cpu,
    Execs,
    Ports,
    Git,
}

impl Column {
    fn def<'a>(
        self,
        sources: &Arc<HashMap<String, WsSources>>,
        fwd: &Gatherer<Option<FwdPorts>>,
    ) -> ColumnDef<Workspace<'a>> {
        match self {
            Column::Name => ColumnDef::new("NAME", Align::Left, |r: &Workspace<'a>| {
                text(r.name.clone())
            }),
            Column::Status => {
                let sources = sources.clone();
                ColumnDef::new("STATUS", Align::Left, move |r: &Workspace<'a>| {
                    value(
                        sources[&r.name].info.cell(|i: &Option<Info>| {
                            i.as_ref().map_or(Datum::Pending, |i| i.status)
                        }),
                    )
                })
            }
            Column::Mem => {
                let sources = sources.clone();
                ColumnDef::new("MEM", Align::Right, move |r: &Workspace<'a>| {
                    value(
                        sources[&r.name]
                            .stats
                            .cell(|s: &Option<Stats>| s.as_ref().map_or(Datum::Pending, |s| s.mem)),
                    )
                })
            }
            Column::Cpu => {
                let sources = sources.clone();
                ColumnDef::new("CPU", Align::Right, move |r: &Workspace<'a>| {
                    value(
                        sources[&r.name]
                            .stats
                            .cell(|s: &Option<Stats>| s.as_ref().map_or(Datum::Pending, |s| s.cpu)),
                    )
                })
            }
            Column::Execs => {
                let sources = sources.clone();
                ColumnDef::new("EXECS", Align::Right, move |r: &Workspace<'a>| {
                    value(sources[&r.name].execs.cell(|e: &Datum<Execs>| *e))
                })
            }
            Column::Ports => {
                let fwd = fwd.clone();
                ColumnDef::new("PORTS", Align::Left, move |r: &Workspace<'a>| {
                    let name = r.name.clone();
                    value(fwd.cell(move |m: &Option<FwdPorts>| {
                        m.as_ref().map_or(Datum::Pending, |m| {
                            let mut ports = m.get(&name).cloned().unwrap_or_default();
                            ports.sort_unstable();
                            Datum::Value(Ports(ports))
                        })
                    }))
                })
            }
            Column::Git => {
                let sources = sources.clone();
                ColumnDef::new("GIT", Align::Left, move |r: &Workspace<'a>| {
                    value(sources[&r.name].git.cell(|g: &Datum<String>| g.clone()))
                })
            }
        }
    }
}

impl Status {
    pub(crate) async fn run(self, project: Option<String>) -> eyre::Result<()> {
        let config = Config::load()?;
        let state = State::new(project, &config).await?;
        let dc = state.try_devcontainer()?;
        let mut workspaces = Workspace::list(&state).await?;

        let docker = dc.docker.clone();

        // One command feeds every workspace's forwarded ports.
        let fwd = spawn_fwd(docker.clone(), state.project_name.to_string());

        let sources: Arc<HashMap<String, WsSources>> = Arc::new(
            workspaces
                .iter()
                .map(|ws| {
                    (
                        ws.name.clone(),
                        build_sources(docker.clone(), ws.path.clone()),
                    )
                })
                .collect(),
        );

        workspaces.sort_by(|a, b| b.is_root.cmp(&a.is_root).then_with(|| a.name.cmp(&b.name)));

        let columns = [
            Column::Name,
            Column::Status,
            Column::Mem,
            Column::Cpu,
            Column::Execs,
            Column::Ports,
            Column::Git,
        ];
        let table = columns
            .into_iter()
            .map(|c| c.def(&sources, &fwd))
            .collect::<TableBuilder<Workspace>>()
            .build(&workspaces, self.live);

        if std::io::stderr().is_terminal() {
            table.run_tty().await
        } else {
            table.run_piped().await
        }
    }
}

/// Forwarded-ports gatherer (one call, all workspaces).
fn spawn_fwd(docker: Arc<DockerClient>, project: String) -> Gatherer<Option<FwdPorts>> {
    Gatherer::spawn(PERIOD, move || {
        let docker = docker.clone();
        let project = project.clone();
        async move { Some(docker.forwarded_ports(&project).await.unwrap_or_default()) }
    })
}

/// The per-workspace gatherers. `stats`/`execs` derive off `info` to reuse the
/// ids it discovers, so each runs independently without re-enumerating.
fn build_sources(docker: Arc<DockerClient>, path: PathBuf) -> WsSources {
    let info = {
        let docker = docker.clone();
        let path = path.clone();
        Gatherer::spawn(PERIOD, move || {
            let docker = docker.clone();
            let path = path.clone();
            async move {
                let containers = docker
                    .container_info_for_path(&path)
                    .await
                    .unwrap_or_default();
                let status = match containers.iter().map(|c| c.state).max() {
                    Some(s) => Datum::Value(ContainerState(s)),
                    None => Datum::NotApplicable,
                };
                let ids = containers.iter().map(|c| c.id.clone()).collect();
                Some(Info { status, ids })
            }
        })
    };

    // Recompute the moment `info` publishes, reusing its ids.
    let stats = {
        let docker = docker.clone();
        let prev: Arc<Mutex<HashMap<String, PrevSample>>> = Arc::new(Mutex::new(HashMap::new()));
        info.derive(move |info| {
            let docker = docker.clone();
            let prev = prev.clone();
            async move { poll_stats(&docker, &info, &prev).await }
        })
    };

    let execs = {
        let docker = docker.clone();
        info.derive(move |info| {
            let docker = docker.clone();
            async move { poll_execs(&docker, &info).await }
        })
    };

    let git = Gatherer::spawn(PERIOD, move || {
        let path = path.clone();
        async move {
            GitStatus::fetch(&path)
                .await
                .map(|g| Datum::Value(g.to_string()))
                .unwrap_or(Datum::NotApplicable)
        }
    });

    WsSources {
        info,
        stats,
        execs,
        git,
    }
}

async fn poll_stats(
    docker: &DockerClient,
    info: &Option<Info>,
    prev: &Mutex<HashMap<String, PrevSample>>,
) -> Option<Stats> {
    // No enumeration yet: stay pending.
    let info = info.as_ref()?;
    if info.ids.is_empty() {
        return Some(Stats {
            mem: Datum::NotApplicable,
            cpu: Datum::NotApplicable,
        });
    }

    let samples = futures::future::join_all(
        info.ids
            .iter()
            .map(|id| async move { (id.clone(), docker.stats_sample(id).await.ok()) }),
    )
    .await;

    let mut prev = prev.lock().unwrap();
    let mut mem_bytes = 0u64;
    let mut cpu_delta = 0u64;
    let mut system_prev = None;
    let mut system_now = None;
    let mut cpus = 1u32;
    let mut have_sample = false;
    for (id, sample) in &samples {
        let Some(sample) = sample else {
            continue;
        };
        have_sample = true;
        mem_bytes += sample.ram;
        if let Some(p) = prev.get(id) {
            cpu_delta += sample.cpu_total.saturating_sub(p.total);
            system_prev = Some(p.system);
        }
        if let Some(system) = sample.system_cpu {
            system_now = Some(system);
        }
        if let Some(c) = sample.online_cpus {
            cpus = c;
        }
        prev.insert(
            id.clone(),
            PrevSample {
                total: sample.cpu_total,
                system: sample.system_cpu.unwrap_or(0),
            },
        );
    }

    if !have_sample {
        return Some(Stats {
            mem: Datum::NotApplicable,
            cpu: Datum::NotApplicable,
        });
    }

    let cpu = match (system_prev, system_now) {
        (Some(sp), Some(sn)) if sn > sp => Datum::Value(Cpu(cpu_delta as f64 / (sn - sp) as f64
            * f64::from(cpus)
            * 100.0)),
        // Only one sample so far: pending.
        _ => Datum::Pending,
    };

    Some(Stats {
        mem: Datum::Value(Bytes(mem_bytes)),
        cpu,
    })
}

async fn poll_execs(docker: &DockerClient, info: &Option<Info>) -> Datum<Execs> {
    let Some(info) = info.as_ref() else {
        return Datum::Pending;
    };
    if info.ids.is_empty() {
        return Datum::NotApplicable;
    }
    let total: usize = futures::future::join_all(info.ids.iter().map(|id| docker.execs(id)))
        .await
        .into_iter()
        .filter_map(Result::ok)
        .sum();
    Datum::Value(Execs(total))
}
