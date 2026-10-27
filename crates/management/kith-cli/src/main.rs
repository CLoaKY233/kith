//! The `kith` binary: daemon and command line in one executable.
//!
//! Plane: management. Binary. See `docs/architecture.md`.

#![forbid(unsafe_code)]

mod cli;
mod daemon;
mod doctor;
mod duration;
mod exec;
mod render;
#[cfg(test)]
mod tests;

use std::io::Write as _;
use std::process::ExitCode;

use cli::Cmd;
use exec::CliError;

fn main() -> ExitCode {
    let opts = cli::opts().run();
    let result = match &opts.cmd {
        Cmd::Daemon {
            config, verbose, ..
        } => exec::load_config(config.as_deref())
            .and_then(|config| daemon::start(config, *verbose).map_err(CliError)),
        Cmd::Status { .. }
        | Cmd::Create { .. }
        | Cmd::Invite { .. }
        | Cmd::Join { .. }
        | Cmd::Leave { .. }
        | Cmd::Peers { .. }
        | Cmd::Dns(_)
        | Cmd::Exit(_)
        | Cmd::Relay(_)
        | Cmd::Server { .. }
        | Cmd::Wg(_)
        | Cmd::Watch
        | Cmd::Doctor
        | Cmd::Config(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CliError(format!("cannot start runtime: {e}")))
            .and_then(|rt| rt.block_on(exec::run(&opts, &mut std::io::stdout().lock()))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Nothing useful remains to do if stderr itself is gone.
            let _ignored = writeln!(std::io::stderr().lock(), "kith: {error}");
            ExitCode::FAILURE
        }
    }
}
