//! Domain-separation contexts.

use core::fmt;

/// A domain-separation string that binds a hash or signature to one purpose.
///
/// Contexts are hard-coded, globally unique and never derived from input. Convention:
/// `"kith <year-month> <purpose> v<n>"`, for example `"kith 2026-10 ledger op v1"`.
/// Changing a context is a protocol change.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Context(&'static str);

impl Context {
    /// Declares a context. Use only string literals.
    #[must_use]
    pub const fn new(context: &'static str) -> Self {
        Self(context)
    }

    /// The context string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Context({:?})", self.0)
    }
}
