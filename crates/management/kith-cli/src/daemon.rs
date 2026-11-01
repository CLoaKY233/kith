//! Entry point for `kith daemon`.

use kith_config::{Config, LogLevel};
use kith_ipc::EntryKind;
use kith_node::Journal;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;

/// Installs the stderr logger, then runs the daemon until shutdown. Debug builds log at
/// debug level with source locations; release builds follow the config.
pub(crate) fn start(config: Config, verbose: usize) -> Result<(), String> {
    let level = log_filter(config.daemon.log_level, verbose).max(DEBUG_BUILD_FLOOR);
    let journal = Journal::new();
    let printed = tracing_subscriber::fmt::layer()
        .with_file(cfg!(debug_assertions))
        .with_line_number(cfg!(debug_assertions))
        .with_writer(std::io::stderr);
    tracing_subscriber::registry()
        .with(
            printed
                .and_then(JournalLayer(journal.clone()))
                .with_filter(level),
        )
        .try_init()
        .map_err(|e| format!("cannot install the logger: {e}"))?;
    run_daemon(config, journal)
}

/// Copies each log line into the journal, where `kith log` and the UI read it.
struct JournalLayer(Journal);

impl<S: tracing::Subscriber> Layer<S> for JournalLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut text = Text::default();
        event.record(&mut text);
        let metadata = event.metadata();
        self.0.push(EntryKind::Log {
            level: metadata.level().as_str().to_ascii_lowercase(),
            target: metadata.target().to_owned(),
            message: text.0,
        });
    }
}

/// A log line's message, then its fields as `name=value`.
#[derive(Default)]
struct Text(String);

impl Visit for Text {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        if field.name() == "message" {
            let fields = std::mem::take(&mut self.0);
            // Writing to a String cannot fail.
            let _ignored = write!(self.0, "{value:?}{fields}");
        } else {
            let _ignored = write!(self.0, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        use std::fmt::Write as _;
        if field.name() == "message" {
            let fields = std::mem::take(&mut self.0);
            self.0 = format!("{value}{fields}");
        } else {
            let _ignored = write!(self.0, " {}={value}", field.name());
        }
    }
}

const DEBUG_BUILD_FLOOR: LevelFilter = if cfg!(debug_assertions) {
    LevelFilter::DEBUG
} else {
    LevelFilter::OFF
};

/// `-v` and `-vv` raise the configured level to debug and trace; they never lower it.
fn log_filter(configured: LogLevel, verbose: usize) -> LevelFilter {
    let configured = match configured {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    };
    let requested = match verbose {
        0 => LevelFilter::OFF,
        1 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    };
    configured.max(requested)
}

/// Runs the daemon until shutdown.
fn run_daemon(config: Config, journal: Journal) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the runtime: {e}"))?;
    runtime
        .block_on(kith_node::run(config, journal))
        .map_err(|e| format!("daemon failed: {}", error_chain(&e)))
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
