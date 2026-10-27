use core::fmt;
use std::{
    fs, io,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{Error, validate::valid_arg};

/// A program invocation: an argument vector and optional stdin, never a shell line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    program: &'static str,
    args: Vec<String>,
    stdin: Option<String>,
    tolerate: Vec<&'static str>,
}

impl Cmd {
    pub(crate) fn new(program: &'static str) -> Self {
        Self {
            program,
            args: Vec::new(),
            stdin: None,
            tolerate: Vec::new(),
        }
    }

    pub(crate) fn arg(mut self, arg: impl fmt::Display) -> Self {
        self.args.push(arg.to_string());
        self
    }

    pub(crate) fn args<T: fmt::Display>(self, args: impl IntoIterator<Item = T>) -> Self {
        args.into_iter().fold(self, Self::arg)
    }

    pub(crate) fn stdin(mut self, input: String) -> Self {
        self.stdin = Some(input);
        self
    }

    /// Treats a failure whose stderr contains `needle` as "already in the desired state".
    pub(crate) fn tolerate(mut self, needle: &'static str) -> Self {
        self.tolerate.push(needle);
        self
    }

    pub(crate) fn tolerates(&self, stderr: &str) -> bool {
        self.tolerate.iter().any(|n| stderr.contains(n))
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        match self.args.iter().find(|a| !valid_arg(a)) {
            Some(bad) => Err(Error::InvalidArgument(bad.clone())),
            None => Ok(()),
        }
    }

    /// The program name.
    pub fn program(&self) -> &str {
        self.program
    }

    /// The arguments.
    pub fn argv(&self) -> &[String] {
        &self.args
    }

    /// Data written to stdin.
    pub fn input(&self) -> Option<&str> {
        self.stdin.as_deref()
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.program)?;
        self.args.iter().try_for_each(|a| write!(f, " {a}"))
    }
}

/// Captured output of a successful command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

/// The side effects kith performs on the OS.
pub trait System {
    /// Runs `cmd`, failing with [`Error::Command`] on a non-zero exit.
    fn run(&mut self, cmd: &Cmd) -> Result<Output, Error>;
    /// Reads a file, or `None` if it does not exist.
    fn read(&mut self, path: &Path) -> Result<Option<String>, Error>;
    /// Atomically replaces a file, creating parent directories.
    fn write(&mut self, path: &Path, contents: &str) -> Result<(), Error>;
    /// Removes a file; a missing file is not an error.
    fn remove(&mut self, path: &Path) -> Result<(), Error>;
    /// Lists the entries of a directory; a missing directory is empty.
    fn list(&mut self, dir: &Path) -> Result<Vec<PathBuf>, Error>;
    /// Whether `name` is an executable on the search path.
    fn has_program(&mut self, name: &str) -> bool;
}

/// The real operating system.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsSystem;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_owned(),
        source,
    }
}

impl System for OsSystem {
    fn run(&mut self, cmd: &Cmd) -> Result<Output, Error> {
        let spawn_err = |source| Error::Spawn {
            command: cmd.to_string(),
            source,
        };
        let mut child = Command::new(cmd.program)
            .args(&cmd.args)
            .env("LC_ALL", "C")
            .stdin(if cmd.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn_err)?;
        if let (Some(input), Some(mut pipe)) = (&cmd.stdin, child.stdin.take()) {
            pipe.write_all(input.as_bytes()).map_err(spawn_err)?;
        }
        let out = child.wait_with_output().map_err(spawn_err)?;
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() {
            return Err(Error::Command {
                command: cmd.to_string(),
                code: out.status.code(),
                stderr,
            });
        }
        Ok(Output {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr,
        })
    }

    fn read(&mut self, path: &Path) -> Result<Option<String>, Error> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(path)(e)),
        }
    }

    fn write(&mut self, path: &Path, contents: &str) -> Result<(), Error> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let tmp = path.with_extension("kith-tmp");
        fs::write(&tmp, contents).map_err(io_err(&tmp))?;
        match fs::rename(&tmp, path) {
            Err(e) if e.kind() == io::ErrorKind::ResourceBusy => {
                fs::remove_file(&tmp).map_err(io_err(&tmp))?;
                fs::write(path, contents).map_err(io_err(path))
            }
            other => other.map_err(io_err(path)),
        }
    }

    fn remove(&mut self, path: &Path) -> Result<(), Error> {
        match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(path)(e)),
            _ => Ok(()),
        }
    }

    fn list(&mut self, dir: &Path) -> Result<Vec<PathBuf>, Error> {
        match fs::read_dir(dir) {
            Ok(entries) => entries
                .map(|e| e.map(|e| e.path()).map_err(io_err(dir)))
                .collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io_err(dir)(e)),
        }
    }

    fn has_program(&mut self, name: &str) -> bool {
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .chain(["/usr/sbin", "/sbin", "/usr/bin", "/bin"].map(PathBuf::from))
            .any(|dir| dir.join(name).is_file())
    }
}

/// A system that performs nothing: every command succeeds, and the queries kith makes get
/// the answers of a plain host (sysctls off, a default route via `eth0` or `en0`).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopSystem;

impl System for NoopSystem {
    fn run(&mut self, cmd: &Cmd) -> Result<Output, Error> {
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        let stdout = match (cmd.program, args.as_slice()) {
            ("sysctl", ["-n", _]) => "0\n",
            ("ip", [_, "route", "show", "default"]) => "default via 192.0.2.1 dev eth0\n",
            ("route", ["-n", "get", ..]) => "gateway: 192.0.2.1\ninterface: en0\n",
            ("pfctl", ["-E"]) => "pf enabled\nToken : 1\n",
            _ => "",
        };
        Ok(Output {
            stdout: stdout.to_owned(),
            stderr: String::new(),
        })
    }

    fn read(&mut self, _path: &Path) -> Result<Option<String>, Error> {
        Ok(None)
    }

    fn write(&mut self, _path: &Path, _contents: &str) -> Result<(), Error> {
        Ok(())
    }

    fn remove(&mut self, _path: &Path) -> Result<(), Error> {
        Ok(())
    }

    fn list(&mut self, _dir: &Path) -> Result<Vec<PathBuf>, Error> {
        Ok(Vec::new())
    }

    fn has_program(&mut self, _name: &str) -> bool {
        true
    }
}

/// The process's effective user id, or `None` on non-Unix platforms.
#[cfg(unix)]
#[expect(unsafe_code, reason = "geteuid has no safe std wrapper")]
#[must_use]
pub fn effective_uid() -> Option<u32> {
    // SAFETY: geteuid takes no arguments, cannot fail and touches no memory.
    Some(unsafe { libc::geteuid() })
}

/// The process's effective user id, or `None` on non-Unix platforms.
#[cfg(not(unix))]
#[must_use]
pub fn effective_uid() -> Option<u32> {
    None
}
