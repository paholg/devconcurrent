//! Rendering for [`Table`](super::Table). The redraw loop that drives this
//! lives in [`Report`](super::Report), which may hold several tables.

use console::style;
use tabular::{Row, Table as TabularTable};

use super::{CellState, Report, Table};

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

impl Table {
    /// True once no cell is pending.
    pub(super) fn done(&self) -> bool {
        self.grid
            .iter()
            .flatten()
            .all(|c| matches!(c.get(), CellState::Ready(_)))
    }

    /// Render the table to a string. Pending cells show the spinner frame, or
    /// `-` on the final frame.
    pub(super) fn render_block(&self, frame: usize, final_frame: bool, width: u16) -> String {
        let spec = self
            .headers
            .iter()
            .map(|(_, align)| align.spec())
            .collect::<Vec<_>>()
            .join("  ");
        let mut table = TabularTable::new(&spec);

        let mut header = Row::new();
        for (h, _) in &self.headers {
            header.add_cell(*h);
        }
        table.add_row(header);

        let spinner = style(SPINNER[frame % SPINNER.len()])
            .black()
            .bright()
            .to_string();
        for cells in &self.grid {
            let mut row = Row::new();
            for cell in cells {
                let s = match cell.get() {
                    CellState::Ready(s) => s,
                    CellState::Pending if final_frame => super::dash(),
                    CellState::Pending => spinner.clone(),
                };
                row.add_ansi_cell(s);
            }
            table.add_row(row);
        }

        let rendered = table.to_string();
        let mut out = String::new();
        for line in rendered.lines() {
            out.push_str(&console::truncate_str(line, width.into(), ""));
            out.push('\n');
        }
        out
    }

    /// Render a final, static frame to a string — for callers that already have
    /// all their data and just want the aligned output (no spinner, no
    /// truncation, no cursor control).
    pub(crate) fn rendered(&self) -> String {
        self.render_block(0, true, u16::MAX)
    }
    /// Redraw in place if stdout is a terminal; otherwise print once.
    pub(crate) async fn run(self) -> eyre::Result<()> {
        Report::from(self).run().await
    }
}
