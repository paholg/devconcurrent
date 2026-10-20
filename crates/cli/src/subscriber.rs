use std::fmt::Write as _;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use console::{Color, style};
use indicatif::ProgressStyle;
use jiff::fmt::friendly::SpanPrinter;
use jiff::{Unit, Zoned};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_indicatif::IndicatifLayer;
use tracing_indicatif::filter::IndicatifFilter;
use tracing_indicatif::writer::{IndicatifWriter, Stderr, Stdout};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

/// The target for a line a child wrote to its stdout, forwarded to ours.
pub(crate) const CHILD_STDOUT: &str = "devconcurrent::child::stdout";
/// The target for a line a child wrote to its stderr, forwarded to ours.
pub(crate) const CHILD_STDERR: &str = "devconcurrent::child::stderr";

fn ts(time: &Zoned) -> String {
    let ts = time.strftime("%F %T");
    style(ts).black().bright().for_stderr().to_string()
}

/// The writer everything must go through: it suspends the progress bars for the
/// duration of a write, so their next redraw can't clobber it — and, once the
/// last span closes, so they aren't left stranded in the scrollback.
static STDERR: OnceLock<IndicatifWriter<Stderr>> = OnceLock::new();

/// Print a line to stderr, minding the progress bars if the subscriber is up.
pub(crate) fn eprintln(line: &str) {
    match STDERR.get() {
        // One `writeln!` is one `write_fmt`, which the writer wraps in a single
        // suspend of the progress bars.
        Some(stderr) => {
            let _ = writeln!(stderr.clone(), "{line}");
        }
        None => std::eprintln!("{line}"),
    }
}

pub(crate) fn init_subscriber() {
    let indicatif_layer = IndicatifLayer::new().with_progress_style(
        ProgressStyle::with_template("{span_child_prefix}{spinner} {elapsed} {msg}")
            .expect("invalid progress style template"),
    );
    let stderr_writer = indicatif_layer.get_stderr_writer();
    let stdout_writer = indicatif_layer.get_stdout_writer();
    let _ = STDERR.set(stderr_writer.clone());
    let indicatif_layer = indicatif_layer.with_filter(IndicatifFilter::new(false));

    let dc_layer = DcLayer {
        stderr: stderr_writer,
        stdout: stdout_writer,
    }
    .with_filter(filter_fn(|meta| {
        // Filter out verbose (TRACE) output from dependencies.
        *meta.level() < tracing::Level::DEBUG || meta.target().starts_with("devconcurrent")
    }));

    tracing_subscriber::registry()
        .with(dc_layer)
        .with(indicatif_layer)
        .init();
}

struct HasIndicatif;
struct IndicatifName(String);

struct SpanTiming {
    name: Option<String>,
    description: Option<String>,
    message: Option<String>,
    finish_message: Option<String>,
    start: Zoned,
    entered: AtomicBool,
    /// Set through `run::mark_failed` when the span's work returned an error.
    failed: AtomicBool,
}

struct DcLayer {
    stderr: IndicatifWriter<Stderr>,
    stdout: IndicatifWriter<Stdout>,
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for DcLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };

        let mut visitor = Visitor::default();
        attrs.record(&mut visitor);

        if visitor.indicatif_show {
            span.extensions_mut().insert(HasIndicatif);
        }

        if span.name() == "parallel"
            && let Some(ref name) = visitor.name
        {
            span.extensions_mut().insert(IndicatifName(name.clone()));
        }

        span.extensions_mut().insert(SpanTiming {
            name: visitor.name,
            description: visitor.description,
            message: visitor.message,
            finish_message: visitor.finish_message,
            start: Zoned::now(),
            entered: AtomicBool::new(false),
            failed: AtomicBool::new(visitor.failed),
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };

        let mut visitor = Visitor::default();
        values.record(&mut visitor);

        if visitor.failed
            && let Some(timing) = span.extensions().get::<SpanTiming>()
        {
            timing.failed.store(true, Ordering::Relaxed);
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let extensions = span.extensions();
        let Some(timing) = extensions.get::<SpanTiming>() else {
            return;
        };

        if timing.entered.swap(true, Ordering::Relaxed) {
            return;
        }

        let mut line = ts(&Zoned::now());
        if let Some(ref name) = timing.name {
            let _ = write!(line, " [{name}]");
        }
        if let Some(ref message) = timing.message {
            let _ = write!(line, " {message}:");
        }
        if let Some(ref description) = timing.description {
            let _ = write!(line, " {description}");
        }
        let _ = writeln!(self.stderr.clone(), "{line}");
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let extensions = span.extensions();
        let Some(timing) = extensions.get::<SpanTiming>() else {
            return;
        };

        let now = Zoned::now();
        let mut line = ts(&now);
        if let Some(ref name) = timing.name {
            let _ = write!(line, " [{name}]");
        }

        let dur = timing
            .start
            .duration_until(&now)
            .round(Unit::Millisecond)
            .unwrap();
        let dur = SpanPrinter::new().duration_to_string(&dur);
        if timing.failed.load(Ordering::Relaxed) {
            // The finish message would claim success, which is exactly what the
            // reader must not believe here.
            let red = console::Style::new().red().bright().for_stderr();
            let _ = write!(
                line,
                " {} after {}",
                red.apply_to("Failed"),
                red.apply_to(dur)
            );
        } else {
            if let Some(msg) = &timing.finish_message {
                let _ = write!(line, " {msg}");
            }
            let _ = write!(line, " Took {}", style(dur).green().bright().for_stderr());
        }
        let _ = writeln!(self.stderr.clone(), "{line}");
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        let msg = visitor.message.unwrap_or_default();

        // Find indicatif name from ancestor spans
        let name = ctx.event_span(event).and_then(|span| {
            span.scope()
                .find_map(|s| s.extensions().get::<IndicatifName>().map(|n| n.0.clone()))
        });

        let target = event.metadata().target();
        // Forwarded output goes out _almost_ undecorated, on the stream the
        // child wrote it to. The caveat is that when it's from parallel
        // commands, it'll be interleaved, so we show the source.
        if target == CHILD_STDOUT || target == CHILD_STDERR {
            let line = match &name {
                Some(name) => format!("[{name}] {msg}"),
                None => msg,
            };
            let _ = if target == CHILD_STDOUT {
                writeln!(self.stdout.clone(), "{line}")
            } else {
                writeln!(self.stderr.clone(), "{line}")
            };
            return;
        }

        let level = *event.metadata().level();
        let level_color = match level {
            tracing::Level::ERROR => Color::Red,
            tracing::Level::WARN => Color::Yellow,
            tracing::Level::INFO => Color::Green,
            tracing::Level::DEBUG | tracing::Level::TRACE => Color::Blue,
        };
        let level = style(format!("{level:>5}"))
            .fg(level_color)
            .bright()
            .for_stderr();

        let mut line = ts(&Zoned::now());
        let _ = write!(line, " {level}");
        if let Some(name) = &name {
            let _ = write!(line, " [{}]", style(name).magenta().bright().for_stderr());
        }
        let _ = write!(line, " {msg}");

        let _ = writeln!(self.stderr.clone(), "{line}");
    }
}

// -- Visitor -----------------------------------------------------------------

#[derive(Default)]
struct Visitor {
    name: Option<String>,
    description: Option<String>,
    message: Option<String>,
    finish_message: Option<String>,
    indicatif_show: bool,
    failed: bool,
}

impl Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "name" => self.name = Some(format!("{value:?}")),
            "description" => self.description = Some(format!("{value:?}")),
            "message" => self.message = Some(format!("{value:?}")),
            "finish_message" => self.finish_message = Some(format!("{value:?}")),
            _ => {}
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        match field.name() {
            "indicatif.pb_show" => self.indicatif_show = value,
            "failed" => self.failed = value,
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "name" => self.name = Some(value.to_string()),
            "description" => self.description = Some(value.to_string()),
            "message" => self.message = Some(value.to_string()),
            "finish_message" => self.finish_message = Some(value.to_string()),
            _ => {}
        }
    }
}
