//! The Console's JSON: people, passkeys, authority prompts, device codes and history.
//! See "Console" in `docs/plans/kith-ui-api.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A person's role in the network's ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum PersonRole {
    /// Everything, including people, roles and consoles.
    Owner,
    /// Devices, invites, names, policy and members.
    Admin,
    /// Their own devices.
    Member,
}

impl PersonRole {
    /// Higher is more: member 0, admin 1, owner 2.
    pub const fn rank(self) -> u8 {
        match self {
            Self::Member => 0,
            Self::Admin => 1,
            Self::Owner => 2,
        }
    }

    /// The name the ledger and the database use.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    /// The role [`PersonRole::as_str`] names.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

/// Where a person stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum PersonState {
    /// In the ledger.
    Active,
    /// Has a registration link and no passkey yet.
    Invited,
    /// Made a passkey; waits for an owner or admin to approve it (`add_account`).
    Registered,
}

/// One person, in the ledger or invited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PersonView {
    /// The account id (hex) once active; `p-` and hex while invited.
    pub id: String,
    /// Their name.
    pub name: String,
    /// Their role, or the one they will get.
    pub role: PersonRole,
    /// Where they stand.
    pub state: PersonState,
    /// Their devices' ids.
    pub devices: Vec<String>,
    /// How many passkeys they have.
    pub passkeys: u32,
    /// The account id of who invited them, while invited or registered.
    pub invited_by: Option<String>,
    /// When their registration link lapses, while invited.
    pub expires_at_ms: Option<u64>,
    /// Whether this is the signed-in person.
    pub me: bool,
}

/// How this server is reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum ConsoleMode {
    /// `kith ui` on 127.0.0.1.
    #[default]
    Local,
    /// `kith-ui --public <domain>` over HTTPS.
    Public,
}

/// The signed-in person in `GET /api/status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Me {
    /// Their account id.
    pub id: String,
    /// Their name.
    pub name: String,
    /// Their role.
    pub role: PersonRole,
}

/// A network by id and name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NetworkRef {
    /// Its id, hex.
    pub id: String,
    /// Its name.
    pub name: String,
}

/// `GET /api/me`: who is signed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MeView {
    /// How this server is reached.
    pub mode: ConsoleMode,
    /// The signed-in person; `null` for the session the local one-time link started.
    pub person: Option<PersonView>,
    /// The network the Console manages, once there is one.
    pub network: Option<NetworkRef>,
}

/// The options to pass to `navigator.credentials.create` or `.get`, in `WebAuthn`'s own
/// JSON (`PublicKeyCredentialCreationOptionsJSON` or `...RequestOptionsJSON`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CeremonyOptions {
    /// For `PublicKeyCredential.parseCreationOptionsFromJSON` or
    /// `parseRequestOptionsFromJSON`.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub public_key: Value,
}

/// A browser's answer to a passkey prompt: `PublicKeyCredential.toJSON()`, unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CredentialIn {
    /// `AuthenticationResponseJSON`.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub credential: Value,
}

/// `GET /api/setup/<token>`: what the setup page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetupInfo {
    /// The network this device is in; `null` when setup names a new one.
    pub network: Option<NetworkRef>,
    /// When the link lapses.
    pub expires_at_ms: u64,
}

/// `POST /api/setup/begin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetupBeginIn {
    /// The token from `/setup/<token>`.
    pub token: String,
    /// The owner's name.
    pub name: String,
    /// The new network's name; needed only when [`SetupInfo::network`] is `null`.
    #[serde(default)]
    pub network_name: Option<String>,
}

/// `POST /api/setup/finish` and `POST /api/register/finish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TokenCredentialIn {
    /// The token from the link.
    pub token: String,
    /// `RegistrationResponseJSON`: `PublicKeyCredential.toJSON()` after
    /// `navigator.credentials.create`.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub credential: Value,
}

/// The ledger change an authority prompt signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum AuthorityKind {
    /// Adds a person.
    AddAccount,
    /// Removes a person and their devices.
    RemoveAccount,
    /// Changes a person's role.
    SetRole,
    /// Lets a device sign everyday admin changes for people.
    AuthorizeConsole,
    /// Takes that back.
    RevokeConsole,
    /// Adds another passkey to a person.
    AddCredential,
}

/// A passkey prompt that signs one ledger change: pass `public_key` to
/// `navigator.credentials.get`, then send the answer to `POST /api/authority/finish`.
/// Its challenge is the change's op hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AuthorityChallenge {
    /// What is being signed.
    pub change: AuthorityKind,
    /// `PublicKeyCredentialRequestOptionsJSON`, limited to the signer's passkeys.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub public_key: Value,
    /// When the prompt lapses.
    pub expires_at_ms: u64,
}

/// `POST /api/setup/finish`: the owner, now signed in, and the prompt that authorizes
/// this device as the network's Console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetupFinished {
    /// The owner.
    pub person: PersonView,
    /// `authorize_console` for this device.
    pub authority: AuthorityChallenge,
}

/// `POST /api/setup/recovery`: the recovery words, shown once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecoveryWords {
    /// The words, in order.
    pub words: Vec<String>,
}

/// `GET /api/register/<token>`: what an invited person's page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RegisterInfo {
    /// Their name.
    pub name: String,
    /// The role they will get.
    pub role: PersonRole,
    /// The network.
    pub network: NetworkRef,
    /// The inviter's name.
    pub invited_by: String,
    /// When the link lapses.
    pub expires_at_ms: u64,
    /// Whether they already made their passkey.
    pub registered: bool,
}

/// `POST /api/register/begin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TokenIn {
    /// The token from the link.
    pub token: String,
}

/// `POST /api/people`: invites a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PersonIn {
    /// Their name.
    pub name: String,
    /// Their role; only an owner invites an owner or an admin.
    pub role: PersonRole,
}

/// A new person's one-time registration link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PersonInvite {
    /// The invited person.
    pub person: PersonView,
    /// `<origin>/register/<token>`; works once.
    pub link: String,
    /// When it lapses.
    pub expires_at_ms: u64,
    /// The link as a QR code.
    pub qr_svg: Option<String>,
}

/// `POST /api/people/<id>/role`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RoleIn {
    /// The new role.
    pub role: PersonRole,
}

/// `POST /api/authority/begin`: the change to sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "snake_case"))]
pub enum AuthorityChangeIn {
    /// An invited person, of any role (`add_person` is accepted too). Before they made
    /// their passkey, the prompt confirms the invite: an admin's or owner's registration
    /// link works only once confirmed. After, it signs their `AddAccount`.
    #[serde(alias = "add_person")]
    AddAdmin {
        /// The pending person's id.
        person: String,
    },
    /// Removes a person and their devices (`remove_account` is accepted too).
    #[serde(alias = "remove_account")]
    RemovePerson {
        /// Their account id.
        person: String,
    },
    /// Changes a person's role.
    SetRole {
        /// Their account id.
        person: String,
        /// The new role.
        role: PersonRole,
    },
    /// Authorizes a device as a Console; `null` is this one.
    AuthorizeConsole {
        /// The device id.
        #[serde(default)]
        device: Option<String>,
    },
    /// Takes a Console's authorization back.
    RevokeConsole {
        /// The device id.
        device: String,
    },
}

/// `POST /api/authority/begin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AuthorityBeginIn {
    /// The change to sign.
    pub change: AuthorityChangeIn,
}

/// `POST /api/device-codes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceCodeIn {
    /// Whose device it will be, by account id; `null` is the signed-in person. Only
    /// owners and admins name someone else.
    #[serde(default)]
    pub person: Option<String>,
}

/// A code that lets one device join, for 10 minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceCodeView {
    /// Six characters, such as `K7Q2MX`.
    pub code: String,
    /// When it lapses.
    pub expires_at_ms: u64,
    /// `<origin>/j/<code>`.
    pub link: String,
    /// `kith join <link>`.
    pub command: String,
    /// The link as a QR code.
    pub qr_svg: Option<String>,
    /// Whose device it will be.
    pub person: String,
}

/// `GET /j/<code>`: the invite a device joins with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JoinLink {
    /// A `kith://join/` link for one device, valid 10 minutes.
    pub invite_link: String,
    /// The network's name.
    pub network_name: String,
}

/// A journal entry the Console kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StoredEntry {
    /// The row's id; older pages are asked for with `before`.
    pub id: i64,
    /// The daemon run it belongs to.
    pub boot: String,
    /// The entry, flattened as the event stream sends it.
    #[cfg_attr(feature = "ts", ts(as = "kith_ipc::Entry"))]
    pub entry: Value,
}

/// `GET /api/history/journal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JournalPage {
    /// The entries, newest first.
    pub entries: Vec<StoredEntry>,
    /// `before` for the next, older page; `null` when this one reached the oldest.
    pub next_before: Option<i64>,
}

/// Bytes with one device in one minute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TrafficMinute {
    /// The minute's start.
    pub minute_ms: u64,
    /// Received from the device in that minute.
    pub rx_bytes: u64,
    /// Sent to it.
    pub tx_bytes: u64,
}

/// `GET /api/history/traffic`: one per device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TrafficHistory {
    /// The network id.
    pub network: String,
    /// The device id.
    pub node: String,
    /// Minutes with traffic, oldest first; quiet minutes are left out.
    pub minutes: Vec<TrafficMinute>,
}
