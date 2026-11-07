//! The Console's own data: sessions, `WebAuthn` ceremonies, one-time links and codes, and the
//! history the daemon does not keep.
//!
//! Memory, never authority: the ledger decides who is in the network and who may change
//! it. Deleting the database signs everyone out and loses history, nothing else.
//!
//! Everything goes through [`Storage`], so another database can be added later; today
//! [`SqliteStore`] is the only one. Secrets (session cookies, setup and registration
//! tokens) are stored only as their SHA-256, see [`secret_id`].

mod sqlite;

use core::fmt;

use sha2::{Digest as _, Sha256};

pub use sqlite::SqliteStore;

/// How long history is kept: 30 days.
pub const RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// A storage failure.
#[derive(Debug)]
pub struct StoreError(String);

impl StoreError {
    /// A failure described by `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "console database: {}", self.0)
    }
}

impl core::error::Error for StoreError {}

/// A signed-in person's session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// [`secret_id`] of the cookie's secret.
    pub id: String,
    /// The person's account id.
    pub account: String,
    /// When it started.
    pub created_ms: u64,
    /// When it was last used.
    pub last_seen_ms: u64,
    /// When it ends unless used before.
    pub expires_ms: u64,
}

/// A `WebAuthn` ceremony waiting for the browser's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeRow {
    /// The challenge as base64url without padding, as `clientDataJSON` carries it.
    pub challenge: String,
    /// What the ceremony is for, such as `login` or `authority`.
    pub purpose: String,
    /// The signed-in person who started it, if any.
    pub account: Option<String>,
    /// What finishing it needs, as JSON.
    pub payload: String,
    /// When it lapses.
    pub expires_ms: u64,
}

/// A short code that hands one device an invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeRow {
    /// Six Crockford base32 characters, uppercase.
    pub code: String,
    /// The network id.
    pub network: String,
    /// The person the device will belong to.
    pub account: String,
    /// The person who made the code.
    pub created_by: String,
    /// When it was made.
    pub created_ms: u64,
    /// When it lapses.
    pub expires_ms: u64,
    /// When it was used, if it was.
    pub used_ms: Option<u64>,
    /// The invite's id, once the code was used.
    pub invite: Option<String>,
    /// The device that joined with it, once known.
    pub node: Option<String>,
}

/// A passkey as the Console keeps it before the ledger does.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredCredential {
    /// The authenticator's credential id.
    pub credential_id: Vec<u8>,
    /// The P-256 public key, SEC1 uncompressed.
    pub public_key: Vec<u8>,
    /// The relying party it was made for: the Console's domain, or `localhost`.
    pub rp_id: String,
}

/// A person invited but not yet in the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPerson {
    /// A random id, `p-` and hex.
    pub id: String,
    /// [`secret_id`] of the registration link's token.
    pub token: String,
    /// The network id.
    pub network: String,
    /// Their name.
    pub name: String,
    /// `owner`, `admin` or `member`.
    pub role: String,
    /// The account that invited them.
    pub invited_by: String,
    /// When the link was made.
    pub created_ms: u64,
    /// When the link lapses.
    pub expires_ms: u64,
    /// When the inviter's passkey confirmed the invite.
    pub confirmed_ms: Option<u64>,
    /// When they made their passkey.
    pub registered_ms: Option<u64>,
    /// Their passkey, once made.
    pub credential: Option<StoredCredential>,
}

/// One journal entry as the Console saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRow {
    /// The row's id, increasing; ignored when adding.
    pub id: i64,
    /// The daemon run the entry belongs to.
    pub boot: String,
    /// Its number in that run.
    pub seq: u64,
    /// When it happened.
    pub at_ms: u64,
    /// Its kind, `snake_case`.
    pub kind: String,
    /// The network it concerns, if one.
    pub network: Option<String>,
    /// The entry flattened as the event stream sends it, as JSON.
    pub entry: String,
}

/// Bytes moved with one peer in one minute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficRow {
    /// The network id.
    pub network: String,
    /// The peer's device id.
    pub node: String,
    /// The minute's start, in Unix milliseconds.
    pub minute_ms: u64,
    /// Received in that minute.
    pub rx_bytes: u64,
    /// Sent in that minute.
    pub tx_bytes: u64,
}

/// What using a one-time link or code found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redeem<T> {
    /// It was good, and is now used.
    Ok(T),
    /// No such link or code.
    Unknown,
    /// It was used before.
    Used,
    /// It lapsed.
    Expired,
}

/// The Console's storage. Every method is short and synchronous; callers pass the time,
/// so expiry follows the server's clock.
pub trait Storage: Send + Sync + fmt::Debug {
    /// Starts a session.
    fn put_session(&self, session: &SessionRow) -> Result<(), StoreError>;
    /// The session with this id, expired or not.
    fn session(&self, id: &str) -> Result<Option<SessionRow>, StoreError>;
    /// Marks a session used and moves its end.
    fn touch_session(&self, id: &str, last_seen_ms: u64, expires_ms: u64)
    -> Result<(), StoreError>;
    /// Ends a session.
    fn delete_session(&self, id: &str) -> Result<(), StoreError>;
    /// Ends every session of a person.
    fn delete_sessions_of(&self, account: &str) -> Result<(), StoreError>;

    /// Keeps a ceremony's challenge.
    fn put_challenge(&self, challenge: &ChallengeRow) -> Result<(), StoreError>;
    /// Removes and returns a challenge, so each is answered once.
    fn take_challenge(&self, challenge: &str) -> Result<Option<ChallengeRow>, StoreError>;

    /// Keeps a new device code.
    fn put_device_code(&self, code: &DeviceCodeRow) -> Result<(), StoreError>;
    /// Uses a device code: once, before it lapses.
    fn redeem_device_code(
        &self,
        code: &str,
        now_ms: u64,
    ) -> Result<Redeem<DeviceCodeRow>, StoreError>;
    /// Notes the invite a used code handed out.
    fn set_device_code_invite(&self, code: &str, invite: &str) -> Result<(), StoreError>;
    /// Codes of `network` used since `since_ms` whose device is not known yet.
    fn unassigned_codes(
        &self,
        network: &str,
        since_ms: u64,
    ) -> Result<Vec<DeviceCodeRow>, StoreError>;
    /// Notes the device that joined with a code.
    fn set_device_code_node(&self, code: &str, node: &str) -> Result<(), StoreError>;

    /// Keeps a new setup token and retires every unused earlier one.
    fn put_setup_token(
        &self,
        token: &str,
        created_ms: u64,
        expires_ms: u64,
    ) -> Result<(), StoreError>;
    /// Whether a setup token is still good, without using it.
    fn check_setup_token(&self, token: &str, now_ms: u64) -> Result<Redeem<u64>, StoreError>;
    /// Uses a setup token: once, before it lapses. `Ok` carries when it lapses.
    fn use_setup_token(&self, token: &str, now_ms: u64) -> Result<Redeem<u64>, StoreError>;

    /// Keeps a newly invited person.
    fn put_pending(&self, person: &PendingPerson) -> Result<(), StoreError>;
    /// The people invited to `network` and not yet added.
    fn pending(&self, network: &str) -> Result<Vec<PendingPerson>, StoreError>;
    /// The invited person whose link's token has this [`secret_id`].
    fn pending_by_token(&self, token: &str) -> Result<Option<PendingPerson>, StoreError>;
    /// Gives an invited person their passkey: once, before the link lapses.
    fn register_pending(
        &self,
        id: &str,
        credential: &StoredCredential,
        now_ms: u64,
    ) -> Result<Redeem<()>, StoreError>;
    /// Notes that the inviter's passkey confirmed an invite; false when there was none.
    fn confirm_pending(&self, id: &str, now_ms: u64) -> Result<bool, StoreError>;
    /// Forgets an invited person; false when there was none.
    fn delete_pending(&self, id: &str) -> Result<bool, StoreError>;

    /// Keeps journal entries; one already kept is skipped.
    fn add_journal(&self, entries: &[JournalRow]) -> Result<(), StoreError>;
    /// Up to `limit` entries before row `before` (or the newest), newest first, of one
    /// network or all.
    fn journal(
        &self,
        before: Option<i64>,
        network: Option<&str>,
        limit: u32,
    ) -> Result<Vec<JournalRow>, StoreError>;
    /// Adds bytes to each row's minute.
    fn add_traffic(&self, rows: &[TrafficRow]) -> Result<(), StoreError>;
    /// Minutes in `since_ms..until_ms`, of one network and device or all, oldest first.
    fn traffic(
        &self,
        network: Option<&str>,
        node: Option<&str>,
        since_ms: u64,
        until_ms: u64,
    ) -> Result<Vec<TrafficRow>, StoreError>;

    /// A setting.
    fn setting(&self, key: &str) -> Result<Option<String>, StoreError>;
    /// Changes a setting.
    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError>;

    /// Drops history older than [`RETENTION_MS`] and everything expired.
    fn prune(&self, now_ms: u64) -> Result<(), StoreError>;
}

/// How a secret is stored: SHA-256, lowercase hex. A copy of the database never holds a
/// cookie or a link that works.
pub fn secret_id(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

/// Lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    crate::sim::hex(bytes)
}
