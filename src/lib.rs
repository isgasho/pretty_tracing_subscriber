use ansi_term::{ANSIGenericString, Color, Style};
use chrono::format::{DelayedFormat, StrftimeItems};
use chrono::Local;
use std::fmt::Write;
use std::path::MAIN_SEPARATOR;
use std::{fmt, io, iter};
use structopt::StructOpt;
use tracing::{Event, Id, Level, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[cfg(not(debug_assertions))]
const DEFAULT_VERBOSITY: u8 = 2;
#[cfg(debug_assertions)]
const DEFAULT_VERBOSITY: u8 = 4;

#[derive(Debug, Clone, StructOpt)]
pub struct Verbosity {
    /// Decreases logging verbosity. Can be specified multiple times
    #[structopt(long = "quiet", short = "q", multiple = true, parse(from_occurrences))]
    quiet: u8,
    /// Increases logging verbosity. Can be specified multiple times
    #[structopt(
        long = "verbose",
        short = "v",
        multiple = true,
        parse(from_occurrences)
    )]
    verbose: u8,
    /// Logging filters in env_logger format
    #[structopt(long = "log", short = "l", env = "SCROOGE_LOG")]
    log_filters: Option<String>,
}

/// Initialises [`tracing_subscriber`] with options from command-line arguments
pub fn init(root_module: &'static str, verbosity: Verbosity) {
    let verbose_format = cfg!(debug_assertions) || verbosity.verbose != 0;

    let registry = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_span_events(FmtSpan::CLOSE)
            .with_writer(io::stderr)
            .event_format(EventFormatter::new(root_module, verbose_format)),
    );

    if let Some(log_filter) = verbosity.log_filters {
        registry.with(EnvFilter::from(log_filter)).init();
    } else {
        let level_filter: LevelFilter = verbosity.into();
        registry.with(level_filter).init();
    }
}

/// Combines the number of occurrences of `--quiet` and `--verbose` flags into a `LevelFilter`
impl Into<LevelFilter> for Verbosity {
    fn into(self) -> LevelFilter {
        match self.verbose.checked_add(DEFAULT_VERBOSITY as u8) {
            Some(v) => match v.checked_sub(self.quiet) {
                Some(1) => LevelFilter::ERROR,
                Some(2) => LevelFilter::WARN,
                Some(3) => LevelFilter::INFO,
                Some(4) => LevelFilter::DEBUG,
                Some(5) => LevelFilter::TRACE,
                Some(6..=std::u8::MAX) => LevelFilter::TRACE,
                Some(0) | None => LevelFilter::OFF,
            },
            None => LevelFilter::TRACE,
        }
    }
}

struct EventFormatter {
    root: &'static str,
    verbose: bool,
}

impl EventFormatter {
    pub fn new(root_module: &'static str, verbose: bool) -> Self {
        Self {
            root: root_module,
            verbose,
        }
    }

    /// Formats the time
    fn time(&self) -> Option<DelayedFormat<StrftimeItems>> {
        if self.verbose {
            Some(Local::now().format("%H:%M:%S%.3f"))
        } else {
            None
        }
    }

    /// Colors the log level
    fn level(&self, event: &Event) -> Option<ANSIGenericString<str>> {
        Some(match *event.metadata().level() {
            Level::ERROR => Color::Red.bold().paint("error:"),
            Level::WARN => Color::Yellow.bold().paint("warning:"),
            Level::INFO => Color::Green.bold().paint("info:"),
            Level::DEBUG => Color::Blue.bold().paint("debug:"),
            Level::TRACE => Color::Purple.bold().paint("trace:"),
        })
    }

    /// Colors the module
    fn module(&self, event: &Event) -> Option<ANSIGenericString<str>> {
        let style = Style::new().bold();
        if !self.verbose || event.metadata().module_path()? == self.root {
            None
        } else if event.metadata().module_path()?.starts_with(self.root) {
            let module_path = event.metadata().module_path()?.get(self.root.len() + 2..)?;
            Some(style.paint(module_path))
        } else {
            Some(style.paint(event.metadata().module_path()?))
        }
    }

    /// Extracts the last part of the filename
    fn file(&self, event: &Event) -> Option<&str> {
        Some(event.metadata().file()?.split(MAIN_SEPARATOR).last()?)
    }

    /// Formats the context, removing any redundant parts.
    fn write_context(
        f: &mut dyn Write,
        module: Option<ANSIGenericString<str>>,
        file: Option<&str>,
        line: Option<u32>,
    ) -> fmt::Result {
        let mut seen = false;

        if let Some(ref module) = module {
            write!(f, "{}", module)?;
            seen = true;
        }
        if let (Some(file), Some(line)) = (file, line) {
            if module.is_some() {
                f.write_char(':')?;
            }
            write!(f, "{}:{}", file, line)?;
            seen = true;
        }

        if seen {
            f.write_char(' ')?;
        }

        Ok(())
    }

    fn write_span<S, N>(
        &self,
        f: &mut dyn Write,
        ctx: &FmtContext<'_, S, N>,
        span: Option<&Id>,
    ) -> fmt::Result
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
        N: for<'writer> FormatFields<'writer> + 'static,
    {
        let bold = Style::new().bold();
        let mut seen = false;

        let span = span
            .and_then(|id| ctx.span(id))
            .or_else(|| ctx.lookup_current());
        let scope = span
            .into_iter()
            .flat_map(|span| span.from_root().chain(iter::once(span)));

        for span in scope {
            if seen {
                f.write_char(':')?;
            }
            write!(f, "{}", bold.paint(span.metadata().name()))?;
            seen = true;
        }

        if seen {
            f.write_char(' ')?;
        }
        Ok(())
    }
}

impl<S, N> FormatEvent<S, N> for EventFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        f: &mut dyn Write,
        e: &Event<'_>,
    ) -> fmt::Result {
        if let Some(time) = self.time() {
            write!(f, "{} ", time)?;
        }

        Self::write_context(f, self.module(e), self.file(e), e.metadata().line())?;

        if self.verbose {
            self.write_span(f, ctx, e.parent())?;
        }

        if let Some(level) = self.level(e) {
            write!(f, "{} ", level)?;
        }

        ctx.format_fields(f, e)?;

        writeln!(f)
    }
}
