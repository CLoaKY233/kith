//! Entry point for `kith daemon`.

/// Runs the daemon until shutdown.
pub(crate) fn run_daemon(config: kith_config::Config) -> Result<(), String> {
    drop(config);
    Err("the daemon is not built into this binary yet".to_owned())
}
