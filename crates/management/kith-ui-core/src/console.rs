//! The Console: people signed in with passkeys, the network they manage, and the
//! history this server keeps for them. The handlers are in `api/auth.rs`,
//! `api/people.rs`, `api/devicecodes.rs` and `api/history.rs`; this module holds what
//! they share.
//!
//! See `docs/plans/console.md` for the design and `docs/plans/kith-ui-api.md` for the
//! HTTP contract.

use core::net::IpAddr;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::http::header::HeaderValue;
use kith_ipc::{Entry, EntryKind, Request, Response, Status, TrafficSample};
use kith_types::{NetworkId, NodeId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::MissedTickBehavior;

use crate::accounts::{AccountInfo, AccountsBackend, acting_as};
use crate::api::{ApiError, lock};
use crate::ipc::Source;
use crate::session::Token;
use crate::store::{
    ChallengeRow, JournalRow, PendingPerson, SessionRow, Storage, TrafficRow, secret_id,
};
use crate::stream::Persist;
use crate::webauthn::{self, Party};
use crate::wire::{ApiErrorCode, NetworkRef, PersonRole, PersonState, PersonView};
use crate::{Shared, json, now_ms};

/// A signed-in person's session lasts this long after its last use.
pub(crate) const SESSION_MS: u64 = 12 * 60 * 60 * 1000;
/// A setup link lasts this long.
pub const SETUP_MS: u64 = 30 * 60 * 1000;
/// A passkey prompt waits this long for its answer.
pub(crate) const CEREMONY_MS: u64 = 5 * 60 * 1000;
/// A device code lasts this long.
pub(crate) const CODE_MS: u64 = 10 * 60 * 1000;
/// A person's registration link lasts this long.
pub(crate) const REGISTER_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// A session is written back at most this often while it is used.
const SLIDE_EVERY_MS: u64 = 60 * 1000;
/// Tries of `/j/<code>` per address per minute.
const CODE_TRIES: usize = 10;
/// Addresses the rate limiter remembers at most.
const LIMITER_ADDRESSES: usize = 4096;
/// Entries and samples queued for the database.
pub(crate) const PERSIST_QUEUE: usize = 8192;
/// How often old history is dropped.
const PRUNE_EVERY: Duration = Duration::from_hours(1);
/// The settings key of the network the Console manages.
const NETWORK_KEY: &str = "network";
/// The settings key that says the first owner exists.
pub(crate) const SETUP_DONE_KEY: &str = "setup_done";
/// The settings key of the newest traffic sample kept, so a restart adds nothing twice.
const TRAFFIC_KEY: &str = "traffic_through_ms";
/// Crockford's base32 alphabet: no I, L, O or U.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// How the server is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// On `127.0.0.1:<port>`, opened by `kith ui`'s one-time link.
    Local,
    /// Over HTTPS at this domain, with passkeys only.
    Public {
        /// The domain, such as `vpn.example.com`.
        domain: String,
    },
}

/// The server's clock: the system's, moved forward in tests.
#[derive(Debug, Default)]
pub(crate) struct Clock {
    skew_ms: AtomicU64,
}

impl Clock {
    pub(crate) fn now_ms(&self) -> u64 {
        now_ms().saturating_add(self.skew_ms.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    pub(crate) fn advance(&self, ms: u64) {
        self.skew_ms.fetch_add(ms, Ordering::Relaxed);
    }
}

/// Who a request is from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Principal {
    /// The person at this computer, by the local one-time link.
    Local,
    /// A person signed in with a passkey.
    Person {
        account: String,
        /// The session's id, to sign out.
        session: String,
    },
}

impl Principal {
    pub(crate) fn account(&self) -> Option<&str> {
        match self {
            Self::Local => None,
            Self::Person { account, .. } => Some(account),
        }
    }
}

/// The relying party id passkeys are made for.
pub(crate) fn rp_id(shared: &Shared) -> &str {
    match &shared.mode {
        Mode::Local => "localhost",
        Mode::Public { domain } => domain,
    }
}

/// The page origins passkey answers may come from.
pub(crate) fn origins(shared: &Shared) -> Vec<String> {
    match &shared.mode {
        Mode::Local => {
            let mut origins = vec![format!("http://localhost:{}", shared.port)];
            origins.extend(
                shared
                    .dev_origins
                    .iter()
                    .filter(|origin| origin.starts_with("http://localhost:"))
                    .cloned(),
            );
            origins
        }
        Mode::Public { domain } => vec![format!("https://{domain}")],
    }
}

/// Where links this server hands out point: the public domain, or `localhost`, the only
/// local name passkeys work on.
pub(crate) fn link_origin(shared: &Shared) -> String {
    match &shared.mode {
        Mode::Local => shared
            .dev_origins
            .first()
            .cloned()
            .unwrap_or_else(|| format!("http://localhost:{}", shared.port)),
        Mode::Public { domain } => format!("https://{domain}"),
    }
}

/// Checks a passkey answer's origin and relying party against this server.
pub(crate) fn with_party<T>(shared: &Shared, check: impl FnOnce(&Party<'_>) -> T) -> T {
    let origins = origins(shared);
    check(&Party {
        rp_id: rp_id(shared),
        origins: &origins,
    })
}

/// 32 random bytes.
pub(crate) fn random_bytes() -> Result<[u8; 32], ApiError> {
    let mut bytes = [0; 32];
    kith_crypto::fill_random(&mut bytes).map_err(|_| ApiError::unexpected())?;
    Ok(bytes)
}

/// A new secret for a cookie or a link, as hex.
pub(crate) fn secret() -> Result<String, ApiError> {
    Token::random()
        .map(|token| token.hex())
        .map_err(|_| ApiError::unexpected())
}

/// A storage failure, as the page sees it.
pub(crate) fn stored<T>(result: Result<T, crate::store::StoreError>) -> Result<T, ApiError> {
    result.map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::Internal,
            error.to_string(),
        )
    })
}

/// A refused passkey answer, or an answer to a prompt that is no longer open.
pub(crate) fn refused(status: StatusCode, message: impl Into<String>) -> ApiError {
    let code = if status == StatusCode::UNAUTHORIZED {
        ApiErrorCode::Unauthorized
    } else {
        ApiErrorCode::InvalidArgument
    };
    ApiError::new(status, code, message)
}

/// Keeps a passkey prompt until it is answered; returns when it lapses.
pub(crate) fn open_ceremony(
    shared: &Shared,
    purpose: &str,
    account: Option<&str>,
    challenge: &str,
    payload: &impl Serialize,
) -> Result<u64, ApiError> {
    let expires_ms = shared.clock.now_ms().saturating_add(CEREMONY_MS);
    stored(shared.store.put_challenge(&ChallengeRow {
        challenge: challenge.to_owned(),
        purpose: purpose.to_owned(),
        account: account.map(str::to_owned),
        payload: serde_json::to_string(payload)?,
        expires_ms,
    }))?;
    Ok(expires_ms)
}

/// Takes the prompt `challenge` answers, once. It must be for `purpose`, still open, and
/// started by `account` when one is given.
pub(crate) fn close_ceremony<P: DeserializeOwned>(
    shared: &Shared,
    purpose: &str,
    challenge: &str,
    account: Option<&str>,
    status: StatusCode,
) -> Result<P, ApiError> {
    let row = stored(shared.store.take_challenge(challenge))?
        .filter(|row| row.purpose == purpose)
        .ok_or_else(|| refused(status, "this passkey prompt is not open; start again"))?;
    if row.expires_ms <= shared.clock.now_ms() {
        return Err(refused(status, "this passkey prompt lapsed; start again"));
    }
    if account.is_some() && row.account.as_deref() != account {
        return Err(refused(status, "this passkey prompt is someone else's"));
    }
    serde_json::from_str(&row.payload).map_err(|_| ApiError::unexpected())
}

/// The daemon's status, or the simulator's.
pub(crate) async fn current_status(shared: &Shared) -> Result<Status, ApiError> {
    match &shared.source {
        Source::Fixture(sim) => sim.status_now(),
        Source::Daemon(daemon) => crate::api::status_of(daemon).await,
    }
}

/// The network the Console manages, its name and this device; `None` before there is
/// one. The first network this device administers is chosen once and remembered.
pub(crate) async fn console_network(
    shared: &Shared,
) -> Result<Option<(NetworkId, String, Option<NodeId>)>, ApiError> {
    let status = current_status(shared).await?;
    let chosen = stored(shared.store.setting(NETWORK_KEY))?;
    let network = chosen
        .and_then(|id| status.networks.iter().find(|n| n.id.to_string() == id))
        .or_else(|| status.networks.iter().find(|n| n.admin))
        .or_else(|| status.networks.first());
    let Some(network) = network else {
        return Ok(None);
    };
    stored(
        shared
            .store
            .set_setting(NETWORK_KEY, &network.id.to_string()),
    )?;
    Ok(Some((network.id, network.name.clone(), status.node)))
}

/// [`console_network`], or 404 before there is one.
pub(crate) async fn network(
    shared: &Shared,
) -> Result<(NetworkId, String, Option<NodeId>), ApiError> {
    console_network(shared)
        .await?
        .ok_or_else(|| ApiError::not_found("this device is in no network yet"))
}

/// Remembers the network setup made or chose.
pub(crate) fn choose_network(shared: &Shared, network: NetworkId) -> Result<(), ApiError> {
    stored(shared.store.set_setting(NETWORK_KEY, &network.to_string()))
}

pub(crate) fn network_ref(id: NetworkId, name: &str) -> NetworkRef {
    NetworkRef {
        id: id.to_string(),
        name: name.to_owned(),
    }
}

/// The signed-in person and their network, as the ledger has them now: 401 for no
/// person, 403 for one the ledger no longer holds.
pub(crate) async fn signed_in(
    shared: &Shared,
    principal: Option<&Principal>,
) -> Result<(NetworkId, AccountInfo), ApiError> {
    let account = principal
        .and_then(Principal::account)
        .ok_or_else(|| refused(StatusCode::UNAUTHORIZED, "sign in with your passkey"))?;
    let (network, _, _) = network(shared).await?;
    let person = shared
        .accounts
        .accounts(network)
        .await?
        .into_iter()
        .find(|one| one.id == account)
        .ok_or_else(|| denied("you are no longer in this network"))?;
    Ok((network, person))
}

pub(crate) fn denied(message: &str) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        ApiErrorCode::PermissionDenied,
        message,
    )
}

/// 403 unless `person` holds at least `role`.
pub(crate) fn require(person: &AccountInfo, role: PersonRole) -> Result<(), ApiError> {
    if person.role.rank() >= role.rank() {
        Ok(())
    } else {
        Err(denied(match role {
            PersonRole::Owner => "only an owner can do this",
            PersonRole::Admin | PersonRole::Member => "only owners and admins can do this",
        }))
    }
}

/// Starts a session for `account` and returns its cookie.
pub(crate) fn start_session(shared: &Shared, account: &str) -> Result<HeaderValue, ApiError> {
    let secret = secret()?;
    let now = shared.clock.now_ms();
    stored(shared.store.put_session(&SessionRow {
        id: secret_id(&secret),
        account: account.to_owned(),
        created_ms: now,
        last_seen_ms: now,
        expires_ms: now.saturating_add(SESSION_MS),
    }))?;
    HeaderValue::from_str(&shared.sessions.set_cookie_value(&secret))
        .map_err(|_| ApiError::unexpected())
}

/// The person a session cookie's secret belongs to, sliding the session forward; `None`
/// for no such session or one that ended.
pub(crate) fn person_session(shared: &Shared, secret: &str) -> Option<Principal> {
    let id = secret_id(secret);
    let session = shared.store.session(&id).ok().flatten()?;
    let now = shared.clock.now_ms();
    if session.expires_ms <= now {
        let _gone = shared.store.delete_session(&id);
        return None;
    }
    if now.saturating_sub(session.last_seen_ms) >= SLIDE_EVERY_MS {
        let _kept = shared
            .store
            .touch_session(&id, now, now.saturating_add(SESSION_MS));
    }
    Some(Principal::Person {
        account: session.account,
        session: id,
    })
}

/// A person in the ledger, as the page shows them.
pub(crate) fn person_view(account: &AccountInfo, me: Option<&str>) -> PersonView {
    PersonView {
        id: account.id.clone(),
        name: account.name.clone(),
        role: account.role,
        state: PersonState::Active,
        devices: account.devices.iter().map(ToString::to_string).collect(),
        passkeys: u32::try_from(account.credentials.len()).unwrap_or(u32::MAX),
        invited_by: None,
        expires_at_ms: None,
        me: me == Some(account.id.as_str()),
    }
}

/// An invited person, as the page shows them.
pub(crate) fn pending_view(person: &PendingPerson) -> PersonView {
    PersonView {
        id: person.id.clone(),
        name: person.name.clone(),
        role: PersonRole::parse(&person.role).unwrap_or(PersonRole::Member),
        state: if person.credential.is_some() {
            PersonState::Registered
        } else {
            PersonState::Invited
        },
        devices: Vec::new(),
        passkeys: u32::from(person.credential.is_some()),
        invited_by: Some(person.invited_by.clone()),
        expires_at_ms: person.credential.is_none().then_some(person.expires_ms),
        me: false,
    }
}

/// A new device code: six Crockford base32 characters, 30 bits.
pub(crate) fn device_code() -> Result<String, ApiError> {
    let bytes = random_bytes()?;
    Ok(bytes
        .iter()
        .take(6)
        .map(|byte| {
            char::from(
                CODE_ALPHABET
                    .get(usize::from(byte % 32))
                    .copied()
                    .unwrap_or(b'0'),
            )
        })
        .collect())
}

/// A code as typed: uppercase, with the letters people confuse mapped as Crockford says.
pub(crate) fn normalize_code(text: &str) -> String {
    text.trim()
        .chars()
        .filter(|c| *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect()
}

/// Tries per address within a minute, for the public code lookup.
#[derive(Debug, Default)]
pub(crate) struct RateLimiter {
    tries: Mutex<HashMap<IpAddr, VecDeque<u64>>>,
}

impl RateLimiter {
    /// Counts a try from `address`; false when it is over the limit.
    pub(crate) fn allow(&self, address: IpAddr, now_ms: u64) -> bool {
        let since = now_ms.saturating_sub(60_000);
        let mut tries = lock(&self.tries);
        if tries.len() >= LIMITER_ADDRESSES {
            tries.retain(|_, times| times.back().is_some_and(|last| *last > since));
        }
        let times = tries.entry(address).or_default();
        while times.front().is_some_and(|first| *first <= since) {
            times.pop_front();
        }
        if times.len() >= CODE_TRIES {
            return false;
        }
        times.push_back(now_ms);
        true
    }
}

/// The task that writes what [`crate::stream::Live`] sees to the database, assigns
/// devices that joined with a code, and drops old history; stopped with the router.
#[derive(Debug)]
pub(crate) struct Worker(AbortHandle);

impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Starts the [`Worker`]. Needs a running tokio runtime.
pub(crate) fn start_worker(
    store: Arc<dyn Storage>,
    accounts: Arc<dyn AccountsBackend>,
    clock: Arc<Clock>,
    queue: mpsc::Receiver<Persist>,
) -> std::io::Result<Worker> {
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| std::io::Error::other("the Console must start inside a tokio runtime"))?;
    let task = runtime.spawn(work(store, accounts, clock, queue));
    Ok(Worker(task.abort_handle()))
}

async fn work(
    store: Arc<dyn Storage>,
    accounts: Arc<dyn AccountsBackend>,
    clock: Arc<Clock>,
    mut queue: mpsc::Receiver<Persist>,
) {
    let mut rollup = Rollup::new(
        store
            .setting(TRAFFIC_KEY)
            .ok()
            .flatten()
            .and_then(|text| text.parse().ok())
            .unwrap_or(0),
    );
    let mut prune = tokio::time::interval(PRUNE_EVERY);
    prune.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = prune.tick() => {
                let _pruned = store.prune(clock.now_ms());
            }
            item = queue.recv() => {
                let Some(item) = item else { return };
                let mut batch = vec![item];
                while batch.len() < 512 {
                    match queue.try_recv() {
                        Ok(more) => batch.push(more),
                        Err(_) => break,
                    }
                }
                keep(&*store, &*accounts, &clock, &mut rollup, batch).await;
            }
        }
    }
}

/// Writes one batch, then assigns the devices it saw join.
async fn keep(
    store: &dyn Storage,
    accounts: &dyn AccountsBackend,
    clock: &Clock,
    rollup: &mut Rollup,
    batch: Vec<Persist>,
) {
    let mut rows = Vec::new();
    let mut joined = Vec::new();
    for item in batch {
        match item {
            Persist::Entry { boot, entry } => {
                if let EntryKind::DeviceJoined { network, node, .. } = &entry.kind {
                    joined.push((*network, *node, joined_invite(&entry)));
                }
                if let Some(row) = journal_row(&boot, &entry) {
                    rows.push(row);
                }
            }
            Persist::Samples(samples) => rollup.add(&samples),
        }
    }
    if !rows.is_empty() {
        let _kept = store.add_journal(&rows);
    }
    let minutes = rollup.take();
    if !minutes.is_empty() && store.add_traffic(&minutes).is_ok() {
        let _kept = store.set_setting(TRAFFIC_KEY, &rollup.through_ms.to_string());
    }
    for (network, node, invite) in joined {
        assign_joined(store, accounts, clock, network, node, invite).await;
    }
}

fn journal_row(boot: &str, entry: &Entry) -> Option<JournalRow> {
    let flat = json::entry(entry).ok()?;
    Some(JournalRow {
        id: 0,
        boot: boot.to_owned(),
        seq: entry.seq,
        at_ms: entry.at_ms,
        kind: flat.get("kind")?.as_str()?.to_owned(),
        network: flat
            .get("network")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        entry: flat.to_string(),
    })
}

/// The invite a joined device used, as hex, when the daemon's `device_joined` names it.
///
/// Read from the entry's JSON so this works whether or not this build's `kith-ipc` has
/// the field yet: `invite` on `DeviceJoined`, as hex or as 32 bytes.
fn joined_invite(entry: &Entry) -> Option<String> {
    let flat = json::entry(entry).ok()?;
    match flat.get("invite")? {
        serde_json::Value::String(text) => Some(text.to_ascii_lowercase()),
        serde_json::Value::Array(bytes) => {
            let bytes: Option<Vec<u8>> = bytes
                .iter()
                .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
                .collect();
            Some(crate::sim::hex(&bytes?))
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::Object(_) => None,
    }
}

/// A device joined: it is the person's whose device code handed out the invite it used.
///
/// With the invite named, exactly that code's person gets it. A daemon that does not name
/// it yet falls back to the one code of the network that handed out an invite in the last
/// ten minutes and has no device, and assigns nothing when there are several. A code is
/// marked as having its device only once the assignment worked.
async fn assign_joined(
    store: &dyn Storage,
    accounts: &dyn AccountsBackend,
    clock: &Clock,
    network: NetworkId,
    node: NodeId,
    invite: Option<String>,
) {
    let since = clock.now_ms().saturating_sub(CODE_MS);
    let Ok(codes) = store.unassigned_codes(&network.to_string(), since) else {
        return;
    };
    let by_invite = invite.is_some();
    let code = match invite {
        Some(invite) => codes
            .into_iter()
            .find(|code| code.invite.as_deref() == Some(invite.as_str())),
        None => <[_; 1]>::try_from(codes).ok().map(|[code]| code),
    };
    let Some(code) = code else {
        return;
    };
    let assign = accounts.assign_device(network, node, code.account.clone());
    let mut assigned = acting_as(Some(code.created_by.clone()), assign)
        .await
        .is_ok();
    // A member may not assign devices, but the device that used their own code's invite
    // is theirs: the Console assigns it on its own authority.
    if !assigned && by_invite && code.created_by == code.account {
        let assign = accounts.assign_device(network, node, code.account.clone());
        assigned = acting_as(None, assign).await.is_ok();
    }
    if assigned {
        let _kept = store.set_device_code_node(&code.code, &node.to_string());
    }
}

/// Turns the daemon's running counters into bytes per peer per minute.
#[derive(Debug)]
struct Rollup {
    /// The newest counters seen per peer: when, received, sent.
    last: BTreeMap<(NetworkId, NodeId), TrafficSample>,
    /// Bytes per peer and minute not yet written.
    pending: BTreeMap<(NetworkId, NodeId, u64), (u64, u64)>,
    /// The newest sample counted before this run started; samples up to it were kept by
    /// an earlier run.
    kept_ms: u64,
    /// The newest sample counted, of any peer, to save as the next run's `kept_ms`.
    through_ms: u64,
}

impl Rollup {
    fn new(through_ms: u64) -> Self {
        Self {
            last: BTreeMap::new(),
            pending: BTreeMap::new(),
            kept_ms: through_ms,
            through_ms,
        }
    }

    /// Counts the bytes since each peer's previous sample in the minute of the new one.
    /// Counters that went down started again from zero.
    fn add(&mut self, samples: &[(NetworkId, NodeId, TrafficSample)]) {
        for (network, node, sample) in samples {
            let key = (*network, *node);
            let previous = self.last.get(&key).copied();
            if previous.is_some_and(|last| sample.at_ms <= last.at_ms) {
                continue;
            }
            self.last.insert(key, *sample);
            let Some(previous) = previous else {
                continue;
            };
            if sample.at_ms <= self.kept_ms {
                continue;
            }
            let delta = |now: u64, before: u64| now.checked_sub(before).unwrap_or(now);
            let minute = sample.at_ms - sample.at_ms % 60_000;
            let bucket = self.pending.entry((*network, *node, minute)).or_default();
            bucket.0 = bucket
                .0
                .saturating_add(delta(sample.rx_bytes, previous.rx_bytes));
            bucket.1 = bucket
                .1
                .saturating_add(delta(sample.tx_bytes, previous.tx_bytes));
            self.through_ms = self.through_ms.max(sample.at_ms);
        }
    }

    /// The minutes to write, with traffic.
    fn take(&mut self) -> Vec<TrafficRow> {
        core::mem::take(&mut self.pending)
            .into_iter()
            .filter(|(_, (rx, tx))| *rx > 0 || *tx > 0)
            .map(
                |((network, node, minute_ms), (rx_bytes, tx_bytes))| TrafficRow {
                    network: network.to_string(),
                    node: node.to_string(),
                    minute_ms,
                    rx_bytes,
                    tx_bytes,
                },
            )
            .collect()
    }
}

/// Makes a single-use invite valid for [`CODE_MS`] in `network`, for a device code.
pub(crate) async fn one_device_invite(
    shared: &Shared,
    network: NetworkId,
) -> Result<String, ApiError> {
    let expires_secs = CODE_MS / 1000;
    match &shared.source {
        Source::Fixture(sim) => {
            let made = sim.create_invite(
                &network.to_string(),
                &crate::wire::InviteIn {
                    expires_secs,
                    uses: Some(1),
                },
            )?;
            made.0
                .get("link")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(ApiError::unexpected)
        }
        Source::Daemon(daemon) => {
            let request = Request::CreateInvite {
                network,
                expires_in_secs: expires_secs,
                max_uses: Some(1),
            };
            let Response::Invite { invite_link } = daemon.ask(request).await? else {
                return Err(ApiError::unexpected());
            };
            Ok(invite_link)
        }
    }
}

/// Signs the first owner: settings say setup happened, so restarts mint no setup link.
pub(crate) fn mark_setup_done(shared: &Shared) -> Result<(), ApiError> {
    stored(shared.store.set_setting(SETUP_DONE_KEY, "1"))
}

/// A `401`, `404` or `410` for a one-time thing that is not good.
pub(crate) fn link_refusal<T>(found: crate::store::Redeem<T>) -> Result<T, ApiError> {
    use crate::store::Redeem;
    match found {
        Redeem::Ok(value) => Ok(value),
        Redeem::Unknown => Err(ApiError::not_found("no such link or code")),
        Redeem::Used => Err(ApiError::new(
            StatusCode::GONE,
            ApiErrorCode::SessionUsed,
            "this link or code was already used",
        )),
        Redeem::Expired => Err(ApiError::new(
            StatusCode::GONE,
            ApiErrorCode::SessionExpired,
            "this link or code lapsed",
        )),
    }
}

/// Decodes a passkey answer the page passed through.
pub(crate) fn credential<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, ApiError> {
    serde_json::from_value(value)
        .map_err(|error| ApiError::bad(format!("not a passkey answer: {error}")))
}

/// A `WebAuthn` refusal as an [`ApiError`].
pub(crate) fn passkey_error(status: StatusCode) -> impl Fn(webauthn::WebAuthnError) -> ApiError {
    move |error| refused(status, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: NetworkId = NetworkId::from_bytes([1; 32]);
    const PEER: NodeId = NodeId::from_bytes([2; 32]);

    fn sample(at_ms: u64, rx_bytes: u64) -> (NetworkId, NodeId, TrafficSample) {
        sample_of(PEER, at_ms, rx_bytes)
    }

    fn sample_of(peer: NodeId, at_ms: u64, rx_bytes: u64) -> (NetworkId, NodeId, TrafficSample) {
        (
            NET,
            peer,
            TrafficSample {
                at_ms,
                rx_bytes,
                tx_bytes: 0,
            },
        )
    }

    #[test]
    fn counters_become_bytes_per_minute() {
        let mut rollup = Rollup::new(0);
        rollup.add(&[
            sample(59_000, 100),
            sample(61_000, 150),
            sample(62_000, 400),
        ]);
        rollup.add(&[sample(61_000, 150), sample(120_500, 20)]);
        let minutes: Vec<(u64, u64)> = rollup
            .take()
            .iter()
            .map(|row| (row.minute_ms, row.rx_bytes))
            .collect();
        assert_eq!(minutes, [(60_000, 300), (120_000, 20)]);
        let mut restarted = Rollup::new(62_000);
        restarted.add(&[
            sample(61_000, 150),
            sample(62_000, 400),
            sample(64_000, 500),
        ]);
        assert_eq!(
            restarted.take().len(),
            1,
            "minutes kept before are not added twice"
        );
        let other = NodeId::from_bytes([3; 32]);
        let mut both = Rollup::new(0);
        both.add(&[
            sample(119_000, 0),
            sample(121_000, 10),
            sample_of(other, 59_000, 0),
            sample_of(other, 61_000, 7),
        ]);
        assert_eq!(
            both.take().len(),
            2,
            "a peer's samples older than another's newest still count"
        );
    }

    fn code(code: &str, account: &str, invite: &str) -> crate::store::DeviceCodeRow {
        crate::store::DeviceCodeRow {
            code: code.to_owned(),
            network: NET.to_string(),
            account: account.to_owned(),
            created_by: account.to_owned(),
            created_ms: 0,
            expires_ms: u64::MAX,
            used_ms: None,
            invite: None,
            node: None,
        }
        .with_invite(invite)
    }

    trait WithInvite {
        fn with_invite(self, invite: &str) -> Self;
    }

    impl WithInvite for crate::store::DeviceCodeRow {
        fn with_invite(mut self, invite: &str) -> Self {
            self.invite = Some(invite.to_owned());
            self
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[expect(clippy::panic_in_result_fn, reason = "assertions fail the test")]
    async fn a_joined_device_goes_to_the_person_whose_code_made_its_invite()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::accounts::MemoryAccounts;
        let store = crate::store::SqliteStore::memory()?;
        let accounts = MemoryAccounts::default();
        let clock = Clock::default();
        let credential = |seed: u8| crate::store::StoredCredential {
            credential_id: vec![seed],
            public_key: vec![seed; 65],
            rp_id: "vpn.example.com".to_owned(),
        };
        let ana = accounts
            .add_account(NET, "Ana".to_owned(), PersonRole::Owner, credential(1))
            .await
            .map_err(|error| error.message().to_owned())?;
        let now = clock.now_ms();
        for (name, invite) in [("AAAAAA", "aa"), ("BBBBBB", "bb")] {
            let row = code(name, &ana.id, invite);
            assert!(store.put_device_code(&row).is_ok());
            assert!(matches!(
                store.redeem_device_code(name, now, invite),
                Ok(crate::store::Redeem::Ok(_))
            ));
        }
        // Two codes out at once: the invite names which one the device used.
        assign_joined(&store, &accounts, &clock, NET, PEER, Some("bb".to_owned())).await;
        let left = store
            .unassigned_codes(&NET.to_string(), 0)
            .unwrap_or_default();
        assert_eq!(
            left.iter().map(|row| row.code.as_str()).collect::<Vec<_>>(),
            ["AAAAAA"]
        );
        let people = accounts.accounts(NET).await.unwrap_or_default();
        assert_eq!(
            people.first().map(|one| one.devices.clone()),
            Some(vec![PEER])
        );
        // Without the invite and with more than one code open, nobody is guessed.
        let other = NodeId::from_bytes([9; 32]);
        let row = code("CCCCCC", &ana.id, "cc");
        assert!(store.put_device_code(&row).is_ok());
        assert!(store.redeem_device_code("CCCCCC", now, "cc").is_ok());
        assign_joined(&store, &accounts, &clock, NET, other, None).await;
        assert_eq!(
            store
                .unassigned_codes(&NET.to_string(), 0)
                .unwrap_or_default()
                .len(),
            2
        );
        Ok(())
    }

    #[test]
    fn codes_read_the_way_people_type_them() {
        assert_eq!(normalize_code(" k7q-2mo "), "K7Q2M0");
        assert_eq!(normalize_code("il"), "11");
    }

    #[test]
    fn the_limiter_counts_per_address_per_minute() {
        let limiter = RateLimiter::default();
        let one = IpAddr::from([10, 0, 0, 1]);
        let two = IpAddr::from([10, 0, 0, 2]);
        for _ in 0..CODE_TRIES {
            assert!(limiter.allow(one, 1_000));
        }
        assert!(!limiter.allow(one, 1_000));
        assert!(limiter.allow(two, 1_000));
        assert!(limiter.allow(one, 61_001));
    }
}
