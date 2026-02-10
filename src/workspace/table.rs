use std::path::Path;

use bollard::secret::ContainerSummaryStateEnum;
use eyre::eyre;
use tabular::{Row, Table};

use crate::{
    bytes::format_bytes,
    workspace::{ExecSession, Workspace},
};

const TABLE_SPEC: &str = "{:<}  {:<}  {:>}  {:>}  {:<}";

fn format_exec(exec: &ExecSession) -> String {
    const MAX_LEN: usize = 40;
    let mut parts = exec.command.iter();
    let first = match parts.next() {
        Some(s) => Path::new(s)
            .file_name()
            .unwrap_or(s.as_ref())
            .to_string_lossy(),
        None => return String::new(),
    };
    let mut out = first.into_owned();
    for arg in parts {
        out.push(' ');
        out.push_str(arg);
    }
    if out.len() > MAX_LEN {
        out.truncate(MAX_LEN - 1);
        out.push('…');
    }
    out
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
}

fn ws_fields(ws: &Workspace) -> eyre::Result<WsFields> {
    let name = ws
        .path
        .file_name()
        .ok_or_else(|| eyre!("workspace path has no filename"))?
        .to_string_lossy();
    let name = if ws.dirty {
        format!("{name}*")
    } else {
        name.into_owned()
    };
    let status = match ws.status() {
        ContainerSummaryStateEnum::EMPTY => "-".to_string(),
        ref s => s.to_string(),
    };
    let mem = if ws.stats.ram == 0 {
        "-".into()
    } else {
        format_bytes(ws.stats.ram)
    };
    Ok(WsFields {
        name,
        status,
        created: format_age(ws.created()),
        mem,
    })
}

fn ws_rows(ws: &Workspace) -> eyre::Result<Vec<Row>> {
    let f = ws_fields(ws)?;
    if ws.execs.is_empty() {
        return Ok(vec![
            Row::new()
                .with_cell(f.name)
                .with_cell(f.status)
                .with_cell(f.created)
                .with_ansi_cell(f.mem)
                .with_cell("-"),
        ]);
    }
    let mut rows = Vec::with_capacity(ws.execs.len());
    for (i, exec) in ws.execs.iter().enumerate() {
        let cmd = format_exec(exec);
        if i == 0 {
            rows.push(
                Row::new()
                    .with_cell(&f.name)
                    .with_cell(&f.status)
                    .with_cell(&f.created)
                    .with_ansi_cell(&f.mem)
                    .with_cell(cmd),
            );
        } else {
            rows.push(
                Row::new()
                    .with_cell("")
                    .with_cell("")
                    .with_cell("")
                    .with_cell("")
                    .with_cell(cmd),
            );
        }
    }
    Ok(rows)
}

/// Full table with header row, for `list` output.
pub fn workspace_table<'a>(
    workspaces: impl IntoIterator<Item = &'a Workspace>,
) -> eyre::Result<Table> {
    let mut table = Table::new(TABLE_SPEC);
    table.add_row(
        Row::new()
            .with_cell("NAME")
            .with_cell("STATUS")
            .with_cell("CREATED")
            .with_cell("MEM")
            .with_cell("EXECS"),
    );
    for ws in workspaces {
        for row in ws_rows(ws)? {
            table.add_row(row);
        }
    }
    Ok(table)
}
