//! The tier-3 gate: before signing the most serious changes (adding or removing an admin,
//! removing this device or leaving, deleting a network, revealing a phone's private
//! config), the person at this computer confirms with the OS itself, so a page left open
//! on an unlocked laptop is not a network left open.
//!
//! macOS asks through `LocalAuthentication` (Touch ID or the login password); Linux through
//! polkit. Until those are wired, every confirmation is unavailable and tier-3 changes
//! are refused.

use core::fmt;

/// Why the OS did not confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsAuthError {
    /// This platform or build has no way to ask.
    Unavailable,
    /// The person refused or cancelled.
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
#[expect(
    clippy::unused_async,
    reason = "the OS prompts it will wait on are asynchronous"
)]
pub async fn confirm(_reason: &str) -> Result<(), OsAuthError> {
    Err(OsAuthError::Unavailable)
}
