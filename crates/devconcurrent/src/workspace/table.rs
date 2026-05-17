use bollard::plugin::ContainerSummaryStateEnum;
use owo_colors::OwoColorize;
use tabular::{Row, Table};

use crate::{bytes::format_bytes, docker::Stats, workspace::git_status::GitStatus};

const FULL_SPEC: &str = "{:<}  {:<}  {:<}  {:>}  {:>}  {:>}  {:<}  {:<}";
const COMPACT_SPEC: &str = "{:<}  {:<}";

pub(crate) struct WorkspaceListRow {
    pub(crate) name: String,
    pub(crate) is_root: bool,
    pub(crate) git_status: GitStatus,
    pub(crate) docker: Option<DockerCells>,
}

pub(crate) struct DockerCells {
    pub(crate) status: ContainerSummaryStateEnum,
    pub(crate) created: Option<i64>,
    pub(crate) dc_managed: bool,
    pub(crate) stats: Stats,
    pub(crate) execs: usize,
    pub(crate) fwd_ports: Vec<u16>,
    pub(crate) docker_ports: Vec<u16>,
}

fn format_age(created: Option<i64>) -> String {
    let ts = match created {
        Some(secs) => jiff::Timestamp::from_second(secs).ok(),
        None => None,
    };
    let ts = match ts {
        Some(t) => t,
        None => return "-".into(),
    };
    let dur = jiff::Timestamp::now().duration_since(ts);
    let secs = dur.as_secs();
    if secs < 0 {
        return "-".into();
    }
    let secs = secs as u64;
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s if s < 604800 => format!("{}d", s / 86400),
        s if s < 2_592_000 => format!("{}w", s / 604800),
        s => format!("{}y", s / 31_536_000),
    }
}

fn status_cell(status: ContainerSummaryStateEnum) -> String {
    match status {
        ContainerSummaryStateEnum::EMPTY => "-".dimmed().to_string(),
        ContainerSummaryStateEnum::RUNNING => status.green().to_string(),
        ContainerSummaryStateEnum::EXITED | ContainerSummaryStateEnum::DEAD => {
            status.red().to_string()
        }
        ContainerSummaryStateEnum::CREATED
        | ContainerSummaryStateEnum::PAUSED
        | ContainerSummaryStateEnum::RESTARTING
        | ContainerSummaryStateEnum::REMOVING => status.yellow().to_string(),
    }
}

fn ports_cell(fwd: &[u16], docker: &[u16]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in fwd {
        parts.push(p.blue().to_string());
    }
    for p in docker {
        parts.push(p.to_string());
    }
    parts.join(",")
}

fn full_row(r: &WorkspaceListRow, d: &DockerCells) -> Row {
    let mem = match d.stats.ram {
        0 => String::new(),
        ram => format_bytes(ram),
    };
    let execs = if d.execs == 0 {
        String::new()
    } else {
        d.execs.to_string()
    };
    let dc = if d.dc_managed { "\u{2713}" } else { "" };
    Row::new()
        .with_cell(&r.name)
        .with_ansi_cell(status_cell(d.status))
        .with_cell(dc)
        .with_cell(format_age(d.created))
        .with_ansi_cell(mem)
        .with_cell(execs)
        .with_ansi_cell(ports_cell(&d.fwd_ports, &d.docker_ports))
        .with_ansi_cell(r.git_status.to_string())
}

fn compact_row(r: &WorkspaceListRow) -> Row {
    Row::new()
        .with_cell(&r.name)
        .with_ansi_cell(r.git_status.to_string())
}

/// Full table with header row, for `list` output.
///
/// When no row has docker info (no devcontainer configured), only NAME and GIT
/// are rendered.
pub(crate) fn workspace_table<'a>(
    workspaces: impl IntoIterator<Item = &'a WorkspaceListRow>,
) -> Table {
    let mut workspaces: Vec<_> = workspaces.into_iter().collect();
    workspaces.sort_by(|a, b| b.is_root.cmp(&a.is_root).then_with(|| a.name.cmp(&b.name)));

    let show_docker = workspaces.iter().any(|r| r.docker.is_some());

    if show_docker {
        let mut table = Table::new(FULL_SPEC);
        table.add_row(
            Row::new()
                .with_cell("NAME")
                .with_cell("STATUS")
                .with_cell("DC")
                .with_cell("CREATED")
                .with_cell("MEM")
                .with_cell("EXECS")
                .with_cell("PORTS")
                .with_cell("GIT"),
        );
        let empty = DockerCells {
            status: ContainerSummaryStateEnum::EMPTY,
            created: None,
            dc_managed: false,
            stats: Stats::default(),
            execs: 0,
            fwd_ports: Vec::new(),
            docker_ports: Vec::new(),
        };
        for r in workspaces {
            let d = r.docker.as_ref().unwrap_or(&empty);
            table.add_row(full_row(r, d));
        }
        table
    } else {
        let mut table = Table::new(COMPACT_SPEC);
        table.add_row(Row::new().with_cell("NAME").with_cell("GIT"));
        for r in workspaces {
            table.add_row(compact_row(r));
        }
        table
    }
}
