//! The tier-3 gate: before signing the most serious changes (adding or removing an admin,
//! removing this device or leaving, deleting a network, revealing a phone's private
//! config), the person at this computer confirms with the OS itself, so a page left open
//! on an unlocked laptop is not a network left open.
//!
//! macOS asks through `LocalAuthentication` (Touch ID or the login password), run by
//! `osascript` so this crate stays free of `unsafe`. Linux asks polkit through `pkcheck`,
//! which shows the desktop's authentication agent. Anything else, a missing agent, an
//! error or a timeout counts as not confirmed: the gate fails closed.

use core::fmt;
use core::time::Duration;

/// How long the person has to answer the OS prompt.
const ANSWER_TIME: Duration = Duration::from_secs(120);

/// Why the OS did not confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsAuthError {
    /// This platform or session has no way to ask.
    Unavailable,
    /// The person refused, cancelled or did not answer in time.
    Denied,
}

impl fmt::Display for OsAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "this computer cannot confirm the change",
            Self::Denied => "the change was not confirmed",
        })
    }
}

impl core::error::Error for OsAuthError {}

/// Asks the person at this computer to confirm `reason`, such as "remove phone from home".
pub async fn confirm(reason: &str) -> Result<(), OsAuthError> {
    let Some(mut command) = prompt(reason) else {
        return Err(OsAuthError::Unavailable);
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(ANSWER_TIME, command.output())
        .await
        .map_err(|_| OsAuthError::Denied)?
        .map_err(|_| OsAuthError::Unavailable)?;
    verdict(output.status.code(), &output.stdout)
}

/// The script `osascript` runs: it answers `confirmed`, `denied` or `unavailable`.
#[cfg(target_os = "macos")]
const MACOS_SCRIPT: &str = r"ObjC.import('LocalAuthentication');
function run(argv) {
  const context = $.LAContext.alloc.init;
  const ownerAuthentication = 2;
  if (!context.canEvaluatePolicyError(ownerAuthentication, null)) return 'unavailable';
  let answer = '';
  context.evaluatePolicyLocalizedReasonReply(ownerAuthentication, argv[0], (ok) => {
    answer = ok ? 'confirmed' : 'denied';
  });
  const until = Date.now() + 120000;
  while (answer === '' && Date.now() < until) {
    $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.1));
  }
  return answer === '' ? 'denied' : answer;
}";

#[cfg(target_os = "macos")]
#[expect(clippy::unnecessary_wraps, reason = "some platforms have no prompt")]
fn prompt(reason: &str) -> Option<tokio::process::Command> {
    let mut command = tokio::process::Command::new("/usr/bin/osascript");
    command.args(["-l", "JavaScript", "-e", MACOS_SCRIPT, reason]);
    Some(command)
}

/// polkit's own action for acting as an administrator: every polkit setup knows it, and
/// it always asks for a password unless the session already holds the right.
#[cfg(target_os = "linux")]
const POLKIT_ACTION: &str = "org.freedesktop.policykit.exec";

#[cfg(target_os = "linux")]
#[expect(clippy::unnecessary_wraps, reason = "some platforms have no prompt")]
fn prompt(_reason: &str) -> Option<tokio::process::Command> {
    let mut command = tokio::process::Command::new("pkcheck");
    command.args([
        "--action-id",
        POLKIT_ACTION,
        "--process",
        &std::process::id().to_string(),
        "--allow-user-interaction",
    ]);
    Some(command)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn prompt(_reason: &str) -> Option<tokio::process::Command> {
    None
}

/// What the prompt's exit code and output mean.
///
/// `osascript` prints its answer and exits 0. `pkcheck` exits 0 when authorized, 1 when
/// not, 3 when the person dismissed the dialog, and anything else when it could not ask.
fn verdict(code: Option<i32>, stdout: &[u8]) -> Result<(), OsAuthError> {
    if cfg!(target_os = "macos") {
        return match (code, String::from_utf8_lossy(stdout).trim()) {
            (Some(0), "confirmed") => Ok(()),
            (Some(0), "denied") => Err(OsAuthError::Denied),
            _ => Err(OsAuthError::Unavailable),
        };
    }
    match code {
        Some(0) => Ok(()),
        Some(1 | 3) => Err(OsAuthError::Denied),
        _ => Err(OsAuthError::Unavailable),
    }
}

/// Who answers the tier-3 question: the OS, or in tests a fixed answer.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Gate {
    /// [`confirm`].
    System,
    /// Always this answer, without asking anyone.
    #[cfg(test)]
    Fixed(Result<(), OsAuthError>),
}

impl Gate {
    /// Asks, as this gate does.
    pub(crate) async fn confirm(self, reason: &str) -> Result<(), OsAuthError> {
        match self {
            Self::System => confirm(reason).await,
            #[cfg(test)]
            Self::Fixed(answer) => answer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clear_yes_confirms() {
        if cfg!(target_os = "macos") {
            assert_eq!(verdict(Some(0), b"confirmed\n"), Ok(()));
            assert_eq!(verdict(Some(0), b"denied\n"), Err(OsAuthError::Denied));
            assert_eq!(
                verdict(Some(0), b"unavailable\n"),
                Err(OsAuthError::Unavailable)
            );
            assert_eq!(verdict(Some(1), b""), Err(OsAuthError::Unavailable));
        } else {
            assert_eq!(verdict(Some(0), b""), Ok(()));
            assert_eq!(verdict(Some(3), b""), Err(OsAuthError::Denied));
            assert_eq!(verdict(Some(127), b""), Err(OsAuthError::Unavailable));
        }
        assert_eq!(verdict(None, b"confirmed"), Err(OsAuthError::Unavailable));
    }
}
