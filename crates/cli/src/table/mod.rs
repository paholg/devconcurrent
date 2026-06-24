//! A small async table abstraction.
//!
//! A `Table` is an erased grid of [`CellSource`]s plus column headers and
//! alignment. Cell values may be known, resolve once, or be fed by a
//! [`Gatherer`]. `live` (keep redrawing) is a property of the table; the data
//! layer produces [`ValueSource`]s and knows nothing about rendering.

use std::fmt::Display;
use std::sync::Arc;

use futures::future::BoxFuture;
use owo_colors::OwoColorize;
use tokio::sync::watch;

pub(crate) mod gatherer;
pub(crate) mod render;

pub(crate) use gatherer::Gatherer;

/// The current content of a cell.
pub(crate) enum CellState {
    /// Shown as a spinner, or `-` once we give up.
    Pending,
    /// Ready to display; may contain ANSI.
    Ready(String),
}

/// The presentation boundary: a pull-only, maybe-not-ready cell value.
pub(crate) trait CellSource: Send {
    fn get(&self) -> CellState;
}

/// A projected value: still loading, not applicable (`-`), or a value.
#[derive(Clone, Copy, Default)]
pub(crate) enum Datum<V> {
    #[default]
    Pending,
    NotApplicable,
    Value(V),
}

#[derive(Clone, Copy)]
pub(crate) enum Align {
    Left,
    Right,
}

impl Align {
    fn spec(self) -> &'static str {
        match self {
            Align::Left => "{:<}",
            Align::Right => "{:>}",
        }
    }
}

/// A typed, maybe-pending value, bridged into a [`CellSource`] by [`value`].
pub(crate) struct ValueSource<V> {
    get: Arc<dyn Fn() -> Datum<V> + Send + Sync>,
    /// Resolves once the source first leaves `Pending` (for the piped path).
    ready: Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>,
}

impl<V> ValueSource<V> {
    /// Backed by a `watch` of `Option<V>` (`None` is pending).
    #[allow(dead_code)]
    pub(crate) fn from_watch(rx: watch::Receiver<Option<V>>) -> Self
    where
        V: Clone + Send + Sync + 'static,
    {
        let get = {
            let rx = rx.clone();
            Arc::new(move || match &*rx.borrow() {
                Some(v) => Datum::Value(v.clone()),
                None => Datum::Pending,
            }) as Arc<dyn Fn() -> Datum<V> + Send + Sync>
        };
        let ready = Arc::new(move || {
            let mut rx = rx.clone();
            Box::pin(async move {
                let _ = rx.wait_for(Option::is_some).await;
            }) as BoxFuture<'static, ()>
        }) as Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;
        ValueSource { get, ready }
    }

    /// Used by [`Gatherer::cell`].
    pub(crate) fn new(
        get: Arc<dyn Fn() -> Datum<V> + Send + Sync>,
        ready: Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>,
    ) -> Self {
        ValueSource { get, ready }
    }
}

/// A grid-ready cell: its erased source and a future that resolves when it
/// first has a value.
pub(crate) struct BuiltCell {
    source: Box<dyn CellSource>,
    ready: BoxFuture<'static, ()>,
}

/// An immediately-available cell.
pub(crate) fn text(s: impl Into<String>) -> BuiltCell {
    struct Static(String);
    impl CellSource for Static {
        fn get(&self) -> CellState {
            CellState::Ready(self.0.clone())
        }
    }
    BuiltCell {
        source: Box::new(Static(s.into())),
        ready: Box::pin(async {}),
    }
}

/// A cell whose value resolves once, via a future.
#[allow(dead_code)]
pub(crate) fn deferred<F>(fut: F) -> BuiltCell
where
    F: std::future::Future<Output = String> + Send + 'static,
{
    let (tx, rx) = watch::channel(None);
    tokio::spawn(async move {
        let v = fut.await;
        let _ = tx.send(Some(v));
    });
    value(ValueSource::from_watch(rx))
}

/// Render a [`ValueSource`] via its [`Display`].
pub(crate) fn value<V>(src: ValueSource<V>) -> BuiltCell
where
    V: Display + 'static,
{
    struct ValueCell<V> {
        get: Arc<dyn Fn() -> Datum<V> + Send + Sync>,
    }
    impl<V: Display> CellSource for ValueCell<V> {
        fn get(&self) -> CellState {
            match (self.get)() {
                Datum::Pending => CellState::Pending,
                Datum::NotApplicable => CellState::Ready(dash()),
                Datum::Value(v) => CellState::Ready(v.to_string()),
            }
        }
    }
    let ready = (src.ready)();
    BuiltCell {
        source: Box::new(ValueCell { get: src.get }),
        ready,
    }
}

/// A column: a header, alignment, and a projection from a row `T` to a cell.
pub(crate) struct ColumnDef<T> {
    header: &'static str,
    align: Align,
    make: Box<dyn Fn(&T) -> BuiltCell>,
}

impl<T> ColumnDef<T> {
    pub(crate) fn new(
        header: &'static str,
        align: Align,
        make: impl Fn(&T) -> BuiltCell + 'static,
    ) -> Self {
        ColumnDef {
            header,
            align,
            make: Box::new(make),
        }
    }
}

/// A set of columns; `collect` from an iterator of [`Column`]s, then `build`.
pub(crate) struct TableBuilder<T> {
    columns: Vec<ColumnDef<T>>,
}

impl<T> FromIterator<ColumnDef<T>> for TableBuilder<T> {
    fn from_iter<I: IntoIterator<Item = ColumnDef<T>>>(iter: I) -> Self {
        TableBuilder {
            columns: iter.into_iter().collect(),
        }
    }
}

impl<T> TableBuilder<T> {
    /// Apply the columns to every row, erasing `T`.
    pub(crate) fn build(self, rows: &[T], live: bool) -> Table {
        let headers: Vec<(&'static str, Align)> =
            self.columns.iter().map(|c| (c.header, c.align)).collect();

        let mut grid: Vec<Vec<Box<dyn CellSource>>> = Vec::with_capacity(rows.len());
        let mut ready: Vec<BoxFuture<'static, ()>> = Vec::new();
        for row in rows {
            let mut cells = Vec::with_capacity(self.columns.len());
            for col in &self.columns {
                let built = (col.make)(row);
                cells.push(built.source);
                ready.push(built.ready);
            }
            grid.push(cells);
        }

        Table {
            headers,
            grid,
            ready,
            live,
        }
    }
}

/// A built, presentation-only table.
pub(crate) struct Table {
    headers: Vec<(&'static str, Align)>,
    grid: Vec<Vec<Box<dyn CellSource>>>,
    ready: Vec<BoxFuture<'static, ()>>,
    live: bool,
}

/// Dimmed placeholder for an unresolved cell.
fn dash() -> String {
    "-".dimmed().to_string()
}
