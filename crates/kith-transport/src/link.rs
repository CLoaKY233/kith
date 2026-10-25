//! Link identity and the events drivers report about links.

use core::time::Duration;

/// Driver-assigned identifier of one established link.
///
/// Drivers must not reuse an identifier while the ladder may still hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkId(pub u64);

/// Ladder-assigned identifier of one attempt, carried by [`crate::Action::Start`].
///
/// Strictly increasing, so the ladder can tell late answers to abandoned attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptId(pub u64);

/// Why a link attempt failed or an established link went down.
///
/// The ladder treats every reason the same; the distinction is for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FailureReason {
    /// No answer before the driver's deadline. Typical of silent blocking.
    Timeout,
    /// Actively refused or reset. Typical of a firewall sending resets.
    Refused,
    /// The handshake reached a peer that did not speak the protocol, e.g. a captive portal or a middlebox.
    ProtocolError,
    /// The local network changed underneath the link.
    NetworkChanged,
    /// The remote side closed the link.
    Closed,
    /// Anything else.
    Other,
}

/// Something a driver observed about a link. Fed to [`crate::Ladder::handle_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkEvent {
    /// An attempt started with [`crate::Action::Start`] produced a working link.
    Up {
        /// Identifier the driver assigned to the link.
        link: LinkId,
        /// The attempt that succeeded.
        attempt: AttemptId,
        /// Handshake round-trip time.
        rtt: Duration,
    },
    /// A link previously reported `Up` stopped working.
    Down {
        /// The link that went down.
        link: LinkId,
        /// Why.
        reason: FailureReason,
    },
    /// An attempt started with [`crate::Action::Start`] gave up without producing a link.
    Failed {
        /// The attempt that failed.
        attempt: AttemptId,
        /// Why.
        reason: FailureReason,
    },
}
