//! Protocol between the `kith` command line and the daemon.
//!
//! Plane: management. Library. See `docs/architecture.md`.
//!
//! Every message is a `kith-wire` envelope ([`kith_wire::registry::IPC_MESSAGE`]) inside a
//! length-prefixed frame. A connection starts with [`Request::Hello`]; the server answers
//! every request with exactly one [`Response`], except that after [`Request::Watch`] it
//! only sends [`Event`]s.

#![forbid(unsafe_code)]

mod client;
mod codec;
mod error;
mod message;
mod server;

pub use client::IpcClient;
pub use error::IpcError;
pub use message::{
    AccountInfo, AccountRole, AccountsInfo, AuthorityChange, BlockedInfo, CredentialInfo,
    DeviceSettingsInfo, DnsRecord, Entry, EntryKind, ErrorCode, Event, GatewayInfo, ImportedInfo,
    InviteInfo, LinkInfo, LinkReport, NetworkSummary, PasskeyAssertion, PasskeyCredential,
    PathKind, PeerInfo, PendingJoin, PolicyInfo, PolicyRuleInfo, Problem, RecordInfo, RelayInfo,
    Request, Response, SelectorInfo, Status, TargetInfo, TrafficSample, TrafficSeries, Warning,
};
pub use server::{Connection, IpcServer, PeerCred};

/// The protocol version both sides must agree on in [`Request::Hello`].
pub const PROTOCOL_VERSION: u32 = 17;

/// The largest request the daemon accepts, in envelope bytes.
pub const MAX_FRAME_LEN: usize = 256 * 1024;

/// The largest response, event or journal entry the client accepts, in envelope bytes.
///
/// Larger than [`MAX_FRAME_LEN`]: an hour of traffic history for a dozen peers is
/// already bigger than that, while requests stay small.
pub const MAX_RESPONSE_LEN: usize = kith_wire::MAX_MESSAGE_LEN;

/// How long the client waits for a response.
pub const REQUEST_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(30);

#[cfg(test)]
mod tests;
