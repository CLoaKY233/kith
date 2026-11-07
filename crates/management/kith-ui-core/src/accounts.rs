//! People in the ledger, as the Console asks for them: one small trait,
//! [`AccountsBackend`], with the daemon behind it ([`IpcAccounts`]) or, under
//! `--fixture` and in tests, a ledger in memory ([`MemoryAccounts`]).
//!
//! The IPC requests (`Accounts`, `AddAccount`, `RemoveAccount`, `SetRole`,
//! `AssignDevice`, `PrepareAuthority`, `SubmitAuthority`, `RecoveryKey`) are added to
//! `kith-ipc` separately. Until then [`Request`] and [`Reply`] here mirror them, and
//! [`IpcAccounts::ask`] is the one place that maps them onto `kith_ipc::Request`.
//! `RemoveAccount` and `SetRole` travel as an [`AuthorityChange`] inside
//! `PrepareAuthority`: both must be signed by a person's passkey.
//!
//! Every everyday admin request the Console sends names the person who asked: the guard
//! runs each request inside [`acting_as`], and [`with_actor`] puts the account on the IPC
//! request.

use core::fmt;
use core::pin::Pin;
use std::collections::BTreeMap;
use std::sync::Mutex;

use axum::http::StatusCode;
use kith_types::{NetworkId, NodeId};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::api::{ApiError, lock};
use crate::ipc::Daemon;
use crate::store::{StoredCredential, hex};
use crate::webauthn::{self, Assertion};
use crate::wire::{ApiErrorCode, PersonRole};

/// One person in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AccountInfo {
    /// BLAKE3 of the first passkey's public key, hex.
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) role: PersonRole,
    pub(crate) credentials: Vec<StoredCredential>,
    pub(crate) devices: Vec<NodeId>,
}

/// A ledger change only a person's passkey may sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AuthorityChange {
    AddAccount {
        name: String,
        role: PersonRole,
        credential: StoredCredential,
    },
    RemoveAccount {
        account: String,
    },
    SetRole {
        account: String,
        role: PersonRole,
    },
    AuthorizeConsole {
        device: NodeId,
    },
    RevokeConsole {
        device: NodeId,
    },
}

/// `PrepareAuthority`'s answer: the op to sign and its hash, the `WebAuthn` challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prepared {
    pub(crate) op_bytes: Vec<u8>,
    pub(crate) challenge: Vec<u8>,
}

/// A passkey's signature over an op, as the ledger checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountAssertion {
    pub(crate) account: String,
    pub(crate) assertion: Assertion,
}

/// The IPC requests about accounts, mirrored until `kith-ipc` has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    Accounts {
        network: NetworkId,
    },
    /// Signed by this device: only the first owner of a network without accounts.
    AddAccount {
        network: NetworkId,
        name: String,
        role: PersonRole,
        credential: StoredCredential,
    },
    AssignDevice {
        network: NetworkId,
        device: NodeId,
        account: String,
    },
    PrepareAuthority {
        network: NetworkId,
        change: AuthorityChange,
    },
    SubmitAuthority {
        network: NetworkId,
        op_bytes: Vec<u8>,
        assertion: AccountAssertion,
    },
    /// The recovery words, only while none were shown.
    RecoveryKey {
        network: NetworkId,
    },
}

/// The daemon's answers to [`Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    Accounts(Vec<AccountInfo>),
    Account(AccountInfo),
    Prepared(Prepared),
    Done,
    RecoveryKey(Vec<String>),
}

/// A boxed answer, so the trait can be used as `dyn`.
pub(crate) type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

/// The ledger's people, as the Console needs them: one request in, one reply out, and
/// typed calls on top.
pub(crate) trait AccountsBackend: Send + Sync + fmt::Debug {
    /// Answers one accounts request.
    fn ask(&self, request: Request) -> Answer<'_, Reply>;

    /// Everyone in `network`.
    fn accounts(&self, network: NetworkId) -> Answer<'_, Vec<AccountInfo>> {
        Box::pin(async move {
            match self.ask(Request::Accounts { network }).await? {
                Reply::Accounts(accounts) => Ok(accounts),
                Reply::Account(_) | Reply::Prepared(_) | Reply::Done | Reply::RecoveryKey(_) => {
                    Err(ApiError::unexpected())
                }
            }
        })
    }

    /// Adds the first owner, signed by this device.
    fn add_account(
        &self,
        network: NetworkId,
        name: String,
        role: PersonRole,
        credential: StoredCredential,
    ) -> Answer<'_, AccountInfo> {
        Box::pin(async move {
            let request = Request::AddAccount {
                network,
                name,
                role,
                credential,
            };
            match self.ask(request).await? {
                Reply::Account(account) => Ok(account),
                Reply::Accounts(_) | Reply::Prepared(_) | Reply::Done | Reply::RecoveryKey(_) => {
                    Err(ApiError::unexpected())
                }
            }
        })
    }

    /// Gives a device to a person.
    fn assign_device(&self, network: NetworkId, device: NodeId, account: String) -> Answer<'_, ()> {
        Box::pin(async move {
            let request = Request::AssignDevice {
                network,
                device,
                account,
            };
            done(&self.ask(request).await?)
        })
    }

    /// The op for `change` and the challenge a passkey signs.
    fn prepare_authority(
        &self,
        network: NetworkId,
        change: AuthorityChange,
    ) -> Answer<'_, Prepared> {
        Box::pin(async move {
            match self
                .ask(Request::PrepareAuthority { network, change })
                .await?
            {
                Reply::Prepared(prepared) => Ok(prepared),
                Reply::Accounts(_) | Reply::Account(_) | Reply::Done | Reply::RecoveryKey(_) => {
                    Err(ApiError::unexpected())
                }
            }
        })
    }

    /// Applies a prepared op with a passkey's signature.
    fn submit_authority(
        &self,
        network: NetworkId,
        op_bytes: Vec<u8>,
        assertion: AccountAssertion,
    ) -> Answer<'_, ()> {
        Box::pin(async move {
            let request = Request::SubmitAuthority {
                network,
                op_bytes,
                assertion,
            };
            done(&self.ask(request).await?)
        })
    }

    /// The recovery words, once.
    fn recovery_key(&self, network: NetworkId) -> Answer<'_, Vec<String>> {
        Box::pin(async move {
            match self.ask(Request::RecoveryKey { network }).await? {
                Reply::RecoveryKey(words) => Ok(words),
                Reply::Accounts(_) | Reply::Account(_) | Reply::Prepared(_) | Reply::Done => {
                    Err(ApiError::unexpected())
                }
            }
        })
    }
}

fn done(reply: &Reply) -> Result<(), ApiError> {
    match reply {
        Reply::Done => Ok(()),
        Reply::Accounts(_) | Reply::Account(_) | Reply::Prepared(_) | Reply::RecoveryKey(_) => {
            Err(ApiError::unexpected())
        }
    }
}

tokio::task_local! {
    /// The account behind the request being served, if a person is signed in.
    static ACTOR: Option<String>;
}

/// Runs `work` on behalf of `actor`.
pub(crate) async fn acting_as<F: Future>(actor: Option<String>, work: F) -> F::Output {
    ACTOR.scope(actor, work).await
}

/// The account behind the request being served.
pub(crate) fn actor() -> Option<String> {
    ACTOR.try_with(Clone::clone).ok().flatten()
}

/// Names the person behind an admin request.
///
/// `kith-ipc` gets an optional `actor: Option<AccountId>` on every admin request; once it
/// has, set it here from [`actor`]. Until then the request goes as it is, signed by this
/// device alone, as before accounts.
pub(crate) const fn with_actor(request: kith_ipc::Request) -> kith_ipc::Request {
    request
}

/// Accounts through the daemon's IPC.
#[derive(Debug)]
pub(crate) struct IpcAccounts {
    daemon: Daemon,
}

impl IpcAccounts {
    pub(crate) const fn new(daemon: Daemon) -> Self {
        Self { daemon }
    }
}

fn request_name(request: &Request) -> &'static str {
    match request {
        Request::Accounts { .. } => "Accounts",
        Request::AddAccount { .. } => "AddAccount",
        Request::AssignDevice { .. } => "AssignDevice",
        Request::PrepareAuthority { .. } => "PrepareAuthority",
        Request::SubmitAuthority { .. } => "SubmitAuthority",
        Request::RecoveryKey { .. } => "RecoveryKey",
    }
}

impl AccountsBackend for IpcAccounts {
    /// The only place that knows the IPC shape: map each [`Request`] onto its
    /// `kith_ipc::Request`, send it with [`Daemon::ask`], and map the answer back onto
    /// [`Reply`]. Until `kith-ipc` has the requests, a daemon that answers at all is told
    /// apart from one that does not keep accounts yet.
    fn ask(&self, request: Request) -> Answer<'_, Reply> {
        Box::pin(async move {
            let _connected = self.daemon.connect().await?;
            Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::VersionSkew,
                format!(
                    "this kith daemon does not keep accounts yet ({})",
                    request_name(&request)
                ),
            ))
        })
    }
}

/// One network's people, consoles and recovery state, as the simulator keeps them.
#[derive(Debug, Default)]
struct Ledger {
    accounts: Vec<AccountInfo>,
    consoles: Vec<NodeId>,
    recovery_shown: bool,
    /// Makes every prepared op unique.
    nonce: u64,
}

/// What [`MemoryAccounts`] signs: the change, for one network, once.
#[derive(Debug, Serialize, Deserialize)]
struct Op {
    network: String,
    nonce: u64,
    change: AuthorityChange,
}

/// Accounts in memory, with the ledger's rules: for `--fixture` and for tests.
///
/// It checks every passkey signature as the ledger does, so a Console that sends the
/// wrong op, challenge or assertion fails here too. Its account ids are SHA-256 based,
/// not BLAKE3: they only have to be stable.
#[derive(Debug, Default)]
pub(crate) struct MemoryAccounts {
    ledgers: Mutex<BTreeMap<NetworkId, Ledger>>,
}

fn denied(message: &str) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        ApiErrorCode::PermissionDenied,
        message,
    )
}

/// The simulator's account id for a first passkey.
fn account_id(public_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"kith account v1");
    hasher.update(public_key);
    hex(&hasher.finalize())
}

/// Words for the simulator's recovery code: not a real key, nothing to recover with.
const WORDS: [&str; 16] = [
    "amber", "birch", "cedar", "delta", "ember", "fjord", "grove", "harbor", "iris", "juniper",
    "kelp", "lumen", "maple", "nectar", "orbit", "pebble",
];

impl MemoryAccounts {
    /// Answers `request` as the ledger would.
    fn answer(&self, request: Request) -> Result<Reply, ApiError> {
        let mut ledgers = lock(&self.ledgers);
        match request {
            Request::Accounts { network } => Ok(Reply::Accounts(
                ledgers
                    .get(&network)
                    .map(|ledger| ledger.accounts.clone())
                    .unwrap_or_default(),
            )),
            Request::AddAccount {
                network,
                name,
                role,
                credential,
            } => {
                let ledger = ledgers.entry(network).or_default();
                if !ledger.accounts.is_empty() {
                    return Err(denied(
                        "the network has people; a passkey must sign new ones",
                    ));
                }
                if role != PersonRole::Owner {
                    return Err(ApiError::bad("the first person is an owner"));
                }
                let account = AccountInfo {
                    id: account_id(&credential.public_key),
                    name,
                    role,
                    credentials: vec![credential],
                    devices: Vec::new(),
                };
                ledger.accounts.push(account.clone());
                Ok(Reply::Account(account))
            }
            Request::AssignDevice {
                network,
                device,
                account,
            } => {
                let ledger = ledgers.entry(network).or_default();
                if !ledger.accounts.iter().any(|one| one.id == account) {
                    return Err(ApiError::not_found("no such person"));
                }
                for one in &mut ledger.accounts {
                    one.devices.retain(|node| *node != device);
                    if one.id == account {
                        one.devices.push(device);
                    }
                }
                Ok(Reply::Done)
            }
            Request::PrepareAuthority { network, change } => {
                let ledger = ledgers.entry(network).or_default();
                ledger.nonce = ledger.nonce.saturating_add(1);
                let op_bytes = serde_json::to_vec(&Op {
                    network: network.to_string(),
                    nonce: ledger.nonce,
                    change,
                })?;
                Ok(Reply::Prepared(Prepared {
                    challenge: Sha256::digest(&op_bytes).to_vec(),
                    op_bytes,
                }))
            }
            Request::SubmitAuthority {
                network,
                op_bytes,
                assertion,
            } => submit(
                ledgers.entry(network).or_default(),
                network,
                &op_bytes,
                &assertion,
            ),
            Request::RecoveryKey { network } => recover(ledgers.entry(network).or_default()),
        }
    }
}

/// Checks a passkey-signed op as the ledger does and applies it.
fn submit(
    ledger: &mut Ledger,
    network: NetworkId,
    op_bytes: &[u8],
    assertion: &AccountAssertion,
) -> Result<Reply, ApiError> {
    let op: Op = serde_json::from_slice(op_bytes)
        .map_err(|_| ApiError::bad("not an op this ledger made"))?;
    if op.network != network.to_string() {
        return Err(ApiError::bad("the op is for another network"));
    }
    let challenge = webauthn::b64(&Sha256::digest(op_bytes));
    let signer = ledger
        .accounts
        .iter()
        .find(|account| account.id == assertion.account)
        .cloned()
        .ok_or_else(|| denied("the signer is not in the network"))?;
    let credential = signer
        .credentials
        .iter()
        .find(|one| one.credential_id == assertion.assertion.credential_id)
        .ok_or_else(|| denied("not one of the signer's passkeys"))?;
    webauthn::verify_assertion(
        &assertion.assertion,
        &credential.public_key,
        &credential.rp_id,
        &challenge,
        None,
    )
    .map_err(|error| denied(&error.to_string()))?;
    apply(ledger, &signer, op.change)?;
    Ok(Reply::Done)
}

/// The simulator's recovery words, once.
fn recover(ledger: &mut Ledger) -> Result<Reply, ApiError> {
    if ledger.recovery_shown {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ApiErrorCode::Conflict,
            "the recovery words were already shown",
        ));
    }
    let mut seed = [0u8; 12];
    kith_crypto::fill_random(&mut seed).map_err(|_| ApiError::unexpected())?;
    ledger.recovery_shown = true;
    Ok(Reply::RecoveryKey(
        seed.iter()
            .map(|byte| {
                WORDS
                    .get(usize::from(byte % 16))
                    .copied()
                    .unwrap_or("amber")
                    .to_owned()
            })
            .collect(),
    ))
}

impl AccountsBackend for MemoryAccounts {
    fn ask(&self, request: Request) -> Answer<'_, Reply> {
        let answer = self.answer(request);
        Box::pin(async move { answer })
    }
}

/// Applies a signed change to the simulator's ledger, with the ledger's role rules.
fn apply(
    ledger: &mut Ledger,
    signer: &AccountInfo,
    change: AuthorityChange,
) -> Result<(), ApiError> {
    let owners = ledger
        .accounts
        .iter()
        .filter(|account| account.role == PersonRole::Owner)
        .count();
    let target_role = |account: &str| {
        ledger
            .accounts
            .iter()
            .find(|one| one.id == account)
            .map(|one| one.role)
            .ok_or_else(|| ApiError::not_found("no such person"))
    };
    match change {
        AuthorityChange::AddAccount {
            name,
            role,
            credential,
        } => {
            if signer.role != PersonRole::Owner
                && (signer.role != PersonRole::Admin || role != PersonRole::Member)
            {
                return Err(denied("only an owner adds owners and admins"));
            }
            let id = account_id(&credential.public_key);
            if ledger.accounts.iter().any(|account| account.id == id) {
                return Err(ApiError::bad("that passkey already belongs to someone"));
            }
            ledger.accounts.push(AccountInfo {
                id,
                name,
                role,
                credentials: vec![credential],
                devices: Vec::new(),
            });
        }
        AuthorityChange::RemoveAccount { account } => {
            let role = target_role(&account)?;
            if signer.role != PersonRole::Owner
                && (signer.role != PersonRole::Admin || role != PersonRole::Member)
            {
                return Err(denied("only an owner removes owners and admins"));
            }
            if role == PersonRole::Owner && owners <= 1 {
                return Err(ApiError::bad("the last owner cannot be removed"));
            }
            ledger.accounts.retain(|one| one.id != account);
        }
        AuthorityChange::SetRole { account, role } => {
            let current = target_role(&account)?;
            if signer.role != PersonRole::Owner {
                return Err(denied("only an owner changes roles"));
            }
            if current == PersonRole::Owner && role != PersonRole::Owner && owners <= 1 {
                return Err(ApiError::bad("the last owner cannot step down"));
            }
            for one in &mut ledger.accounts {
                if one.id == account {
                    one.role = role;
                }
            }
        }
        AuthorityChange::AuthorizeConsole { device } => {
            if signer.role != PersonRole::Owner {
                return Err(denied("only an owner authorizes a console"));
            }
            if !ledger.consoles.contains(&device) {
                ledger.consoles.push(device);
            }
        }
        AuthorityChange::RevokeConsole { device } => {
            if signer.role != PersonRole::Owner {
                return Err(denied("only an owner revokes a console"));
            }
            ledger.consoles.retain(|one| *one != device);
        }
    }
    Ok(())
}
