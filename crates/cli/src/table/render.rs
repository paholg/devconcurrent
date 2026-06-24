//! Rendering for [`Table`](super::Table).
//!
//! - `run_tty`: redraw in place on a tick. Live until Ctrl-C; otherwise until
//!   every cell is ready or the deadline passes, then a final static frame.
//! - `run_piped`: wait for each cell's first value (up to the deadline) and
//!   print once, with no spinner or cursor control.

use std::io::Write;
use std::time::Duration;

use crossterm::{cursor, queue, terminal};
use tabular::{Row, Table as TabularTable};

use super::{CellState, Table};
use crate::ansi::{GRAY, RESET};

/// How long the non-live / piped paths wait before showing `-` for whatever is
/// still pending.
const DEADLINE: Duration = Duration::from_secs(5);

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

const TICK: Duration = Duration::from_millis(80);

impl Table {
    /// True once no cell is pending.
    fn done(&self) -> bool {
        self.grid
            .iter()
            .flatten()
            .all(|c| matches!(c.get(), CellState::Ready(_)))
    }

    /// Render the table to a string and its line count. Pending cells show the
    /// spinner frame, or `-` on the final frame.
    fn render_block(&self, frame: usize, final_frame: bool, width: u16) -> (String, u16) {
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

        let spinner = format!("{GRAY}{}{RESET}", SPINNER[frame % SPINNER.len()]);
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
        let mut lines = 0u16;
        for line in rendered.lines() {
            out.push_str(&truncate_visible(line, width));
            out.push('\n');
            lines += 1;
        }
        (out, lines)
    }

    /// Redraw in place until done (non-live) or Ctrl-C (live).
    pub(crate) async fn run_tty(self) -> eyre::Result<()> {
        let mut stderr = std::io::stderr();
        let mut ticker = tokio::time::interval(TICK);
        let start = tokio::time::Instant::now();
        let mut prev_lines = 0u16;
        let mut frame = 0usize;

        loop {
            if self.live {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = tokio::signal::ctrl_c() => {
                        // Leave the last frame; move below it.
                        writeln!(stderr)?;
                        return Ok(());
                    }
                }
            } else {
                ticker.tick().await;
            }

            let finished = !self.live && (self.done() || start.elapsed() >= DEADLINE);
            let width = terminal::size().map(|(c, _)| c).unwrap_or(u16::MAX);
            let (block, lines) = self.render_block(frame, finished, width);

            if prev_lines > 0 {
                queue!(stderr, cursor::MoveUp(prev_lines))?;
            }
            queue!(stderr, cursor::MoveToColumn(0))?;
            queue!(stderr, terminal::Clear(terminal::ClearType::FromCursorDown))?;
            write!(stderr, "{block}")?;
            stderr.flush()?;

            prev_lines = lines;
            frame += 1;

            if finished {
                return Ok(());
            }
        }
    }

    /// Wait for each cell's first value (up to the deadline), then print once.
    pub(crate) async fn run_piped(mut self) -> eyre::Result<()> {
        let ready = std::mem::take(&mut self.ready);
        let _ = tokio::time::timeout(DEADLINE, futures::future::join_all(ready)).await;

        let (block, _) = self.render_block(0, true, u16::MAX);
        print!("{block}");
        std::io::stdout().flush()?;
        Ok(())
    }
}

/// Truncate to `max` visible columns, copying ANSI escapes verbatim and
/// resetting if cut. Keeps each row one physical line so `MoveUp` stays correct.
fn truncate_visible(line: &str, max: u16) -> String {
    let max = max as usize;
    let mut out = String::new();
    let mut visible = 0usize;
    let mut chars = line.chars().peekable();
    let mut cut = false;

    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            out.push(c);
            while let Some(&next) = chars.peek() {
                chars.next();
                out.push(next);
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if visible >= max {
            cut = true;
            break;
        }
        out.push(c);
        visible += 1;
    }

    if cut {
        out.push_str(&RESET.to_string());
    }
    out
}
