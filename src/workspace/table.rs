use bollard::plugin::ContainerSummaryStateEnum;
use owo_colors::OwoColorize;
use tabular::{Row, Table};

use crate::{bytes::format_bytes, docker::Stats, workspace::git_status::GitStatus};

const TABLE_SPEC: &str = "{:<}  {:<}  {:<}  {:>}  {:>}  {:>}  {:<}  {:<}";

pub(crate) struct WorkspaceListRow {
    pub(crate) name: String,
    pub(crate) is_root: bool,
    pub(crate) git_status: GitStatus,
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
        s if s < 31_536_000 => format!("{}mo", s / 2_592_000),
        s => format!("{}y", s / 31_536_000),
    }
}

struct WsFields {
    name: String,
    status: String,
    created: String,
    mem: String,
    ports: String,
    git: String,
}

fn ws_fields(r: &WorkspaceListRow) -> WsFields {
    let name = r.name.clone();
    let status = match r.status {
        ContainerSummaryStateEnum::EMPTY => "-".dimmed().to_string(),
        ContainerSummaryStateEnum::RUNNING => r.status.green().to_string(),
        ContainerSummaryStateEnum::EXITED | ContainerSummaryStateEnum::DEAD => {
            r.status.red().to_string()
        }
        ContainerSummaryStateEnum::CREATED
        | ContainerSummaryStateEnum::PAUSED
        | ContainerSummaryStateEnum::RESTARTING
        | ContainerSummaryStateEnum::REMOVING => r.status.yellow().to_string(),
    };
    let mem = match r.stats.ram {
        0 => String::new(),
        ram => format_bytes(ram),
    };
    let ports = {
        let mut parts: Vec<String> = Vec::new();
        for p in &r.fwd_ports {
            parts.push(p.blue().to_string());
        }
        for p in &r.docker_ports {
            parts.push(p.to_string());
        }
        parts.join(",")
    };
    WsFields {
        name,
        status,
        created: format_age(r.created),
        mem,
        ports,
        git: r.git_status.to_string(),
    }
}

fn ws_row(r: &WorkspaceListRow) -> Row {
    let f = ws_fields(r);
    let execs = if r.execs == 0 {
        String::new()
    } else {
        r.execs.to_string()
    };
    let dc = if r.dc_managed { "\u{2713}" } else { "" };
    Row::new()
        .with_cell(f.name)
        .with_ansi_cell(f.status)
        .with_cell(dc)
        .with_cell(f.created)
        .with_ansi_cell(f.mem)
        .with_cell(execs)
        .with_ansi_cell(f.ports)
        .with_ansi_cell(f.git)
}

/// Full table with header row, for `list` output.
pub(crate) fn workspace_table<'a>(
    workspaces: impl IntoIterator<Item = &'a WorkspaceListRow>,
) -> Table {
    let mut workspaces: Vec<_> = workspaces.into_iter().collect();
    workspaces.sort_by(|a, b| b.is_root.cmp(&a.is_root).then_with(|| a.name.cmp(&b.name)));

    let mut table = Table::new(TABLE_SPEC);
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
    for ws in workspaces {
        table.add_row(ws_row(ws));
    }
    table
}
