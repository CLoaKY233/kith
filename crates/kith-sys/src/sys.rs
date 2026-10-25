use std::path::{Path, PathBuf};

use crate::{Cmd, Error, Output, Plan, System, linux, macos};

/// A supported operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// Linux with iproute2 and nftables.
    Linux,
    /// macOS.
    MacOs,
}

impl Platform {
    /// The platform this binary runs on; [`Error::Unsupported`] elsewhere (Windows).
    pub fn current() -> Result<Self, Error> {
        if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else if cfg!(target_os = "macos") {
            Ok(Self::MacOs)
        } else {
            Err(Error::Unsupported("only Linux and macOS are supported"))
        }
    }
}

/// Applies and reverts [`Plan`]s on a [`System`].
#[derive(Debug)]
pub struct Sys<S> {
    pub(crate) system: S,
    platform: Platform,
    state_dir: PathBuf,
}

impl<S: System> Sys<S> {
    /// Drives `system` as `platform`, keeping restore data in `state_dir` (e.g. `/var/lib/kith/sys`).
    pub fn new(system: S, platform: Platform, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            system,
            platform,
            state_dir: state_dir.into(),
        }
    }

    /// The underlying system.
    pub fn system(&self) -> &S {
        &self.system
    }

    /// Installs `plan`. Idempotent; a partial failure is undone by [`Sys::revert`].
    pub fn apply(&mut self, plan: &Plan) -> Result<(), Error> {
        match self.platform {
            Platform::Linux => linux::apply(self, plan),
            Platform::MacOs => macos::apply(self, plan),
        }
    }

    /// Removes everything `plan` (or a crashed earlier run) installed, attempting every step.
    pub fn revert(&mut self, plan: &Plan) -> Result<(), Error> {
        let mut errors = Errors::default();
        match self.platform {
            Platform::Linux => linux::revert(self, plan, &mut errors),
            Platform::MacOs => macos::revert(self, plan, &mut errors),
        }
        errors.finish()
    }

    pub(crate) fn exec(&mut self, cmd: &Cmd) -> Result<Output, Error> {
        cmd.validate()?;
        match self.system.run(cmd) {
            Err(Error::Command { stderr, .. }) if cmd.tolerates(&stderr) => Ok(Output::default()),
            other => other,
        }
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "callers build commands inline"
    )]
    pub(crate) fn run(&mut self, cmd: Cmd) -> Result<(), Error> {
        self.exec(&cmd).map(drop)
    }

    pub(crate) fn state(&self, name: &str) -> PathBuf {
        self.state_dir.join(name)
    }

    /// Saves current values of `keys` (unless a previous run already did) and sets them to 1.
    pub(crate) fn enable_sysctls(&mut self, keys: &[&str]) -> Result<(), Error> {
        let path = self.state("sysctl");
        if self.system.read(&path)?.is_none() {
            let mut saved = String::new();
            for key in keys {
                let cmd = Cmd::new("sysctl").arg("-n").arg(key);
                let line = cmd.to_string();
                let value = self.exec(&cmd)?.stdout.trim().to_owned();
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Error::Parse {
                        command: line,
                        output: value,
                    });
                }
                saved.push_str(&lines([format!("{key}={value}")]));
            }
            self.system.write(&path, &saved)?;
        }
        keys.iter()
            .try_for_each(|key| self.run(Cmd::new("sysctl").arg("-w").arg(format!("{key}=1"))))
    }

    pub(crate) fn restore_sysctls(&mut self) -> Result<(), Error> {
        let path = self.state("sysctl");
        let Some(saved) = self.system.read(&path)? else {
            return Ok(());
        };
        for line in saved.lines() {
            self.run(Cmd::new("sysctl").arg("-w").arg(line))?;
        }
        self.system.remove(&path)
    }

    /// Reads a newline-separated state list.
    pub(crate) fn read_list(&mut self, path: &Path) -> Result<Vec<String>, Error> {
        Ok(self
            .system
            .read(path)?
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

/// Joins `items` as newline-terminated lines.
pub(crate) fn lines<T: core::fmt::Display>(items: impl IntoIterator<Item = T>) -> String {
    use core::fmt::Write as _;
    items.into_iter().fold(String::new(), |mut out, item| {
        // Writing to a `String` cannot fail.
        let _ = writeln!(out, "{item}");
        out
    })
}

/// Collects the first error while letting every revert step run.
#[derive(Debug, Default)]
pub(crate) struct Errors(Option<Error>);

impl Errors {
    pub(crate) fn keep(&mut self, result: Result<(), Error>) {
        if let Err(e) = result {
            self.0.get_or_insert(e);
        }
    }

    pub(crate) fn finish(self) -> Result<(), Error> {
        self.0.map_or(Ok(()), Err)
    }
}

/// Parses `key: value` lines (macOS `route get`) or `... key value ...` tokens (iproute2).
pub(crate) fn field<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    let mut tokens = output.split_whitespace();
    while let Some(t) = tokens.next() {
        if t.strip_suffix(':').unwrap_or(t) == key {
            return tokens.next();
        }
    }
    None
}
