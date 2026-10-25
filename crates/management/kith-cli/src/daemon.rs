//! Entry point for `kith daemon`.

use kith_config::{Config, LogLevel};
use tracing_subscriber::filter::LevelFilter;

/// Installs the stderr logger, then runs the daemon until shutdown.
pub(crate) fn start(config: Config, verbose: usize) -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_max_level(log_filter(config.daemon.log_level, verbose))
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|e| format!("cannot install the logger: {e}"))?;
    run_daemon(config)
}

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
fn run_daemon(config: Config) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the runtime: {e}"))?;
    runtime
        .block_on(kith_node::run(config))
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
