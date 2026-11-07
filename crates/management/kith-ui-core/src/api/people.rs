//! `/api/people` and `/api/authority`: who is in the network, and the passkey prompts
//! that change it.
//!
//! Adding a person takes two steps, because a passkey can only sign an op that names the
//! new person's passkey once it exists: an owner or admin invites them (a pending person
//! and a one-time registration link), they make their passkey on `/register/<token>`,
//! and then an owner or admin approves them, which signs their `AddAccount`.
//!
//! Every change to people, roles and consoles is an authority change: `begin` asks the
//! daemon for the op (`PrepareAuthority`) and answers a passkey prompt whose challenge is
//! the op's hash; `finish` checks the answer and hands op and signature to the daemon
//! (`SubmitAuthority`).

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use kith_types::{NetworkId, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Shared;
use crate::accounts::{AccountAssertion, AccountInfo, AuthorityChange};
use crate::api::auth::name;
use crate::api::{Answer, ApiError, accepted, node_id, qr_svg};
use crate::console::{
    Principal, REGISTER_MS, close_ceremony, credential, current_status, denied, link_origin,
    network, open_ceremony, passkey_error, pending_view, person_view, random_bytes, refused,
    require, rp_id, secret, signed_in, stored, with_party,
};
use crate::session::Caller;
use crate::store::{PendingPerson, hex, secret_id};
use crate::webauthn::{Authentication, b64, request_options, unb64, verify_assertion};
use crate::wire::{
    AuthorityBeginIn, AuthorityChallenge, AuthorityChangeIn, AuthorityKind, CredentialIn, PersonIn,
    PersonInvite, PersonRole, RoleIn,
};

/// What to do once an authority change went through.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "then", rename_all = "snake_case")]
pub(crate) enum After {
    Nothing,
    /// The approved person is in the ledger now.
    ForgetPending {
        person: String,
    },
    /// The removed person is signed out everywhere.
    EndSessions {
        account: String,
    },
    /// The inviter confirmed an invite before the person made a passkey.
    ConfirmInvite {
        person: String,
    },
}

/// What an authority prompt needs to finish.
#[derive(Debug, Serialize, Deserialize)]
struct AuthorityPrompt {
    network: String,
    op: String,
    after: After,
}

fn kind(change: &AuthorityChange) -> AuthorityKind {
    match change {
        AuthorityChange::AddAccount { .. } => AuthorityKind::AddAccount,
        AuthorityChange::RemoveAccount { .. } => AuthorityKind::RemoveAccount,
        AuthorityChange::SetRole { .. } => AuthorityKind::SetRole,
        AuthorityChange::AuthorizeConsole { .. } => AuthorityKind::AuthorizeConsole,
        AuthorityChange::RevokeConsole { .. } => AuthorityKind::RevokeConsole,
    }
}

/// Prepares `change` and opens the passkey prompt `signer` answers to sign it.
pub(crate) async fn authority_prompt(
    shared: &Shared,
    network: NetworkId,
    signer: &AccountInfo,
    change: AuthorityChange,
    after: After,
) -> Result<AuthorityChallenge, ApiError> {
    let rp = rp_id(shared).to_owned();
    let passkeys = passkeys_for(signer, &rp)?;
    let change_kind = kind(&change);
    let prepared = shared.accounts.prepare_authority(network, change).await?;
    let challenge = b64(&prepared.challenge);
    let expires_at_ms = open_ceremony(
        shared,
        "authority",
        Some(&signer.id),
        &challenge,
        &AuthorityPrompt {
            network: network.to_string(),
            op: b64(&prepared.op_bytes),
            after,
        },
    )?;
    Ok(AuthorityChallenge {
        change: change_kind,
        public_key: request_options(&rp, &challenge, &passkeys),
        expires_at_ms,
    })
}

/// The prompt by which an inviter confirms an invite before the person has a passkey.
///
/// A passkey can only sign an `AddAccount` that names the new person's passkey, so this
/// signs a random challenge: it proves the inviter was present when the link was made.
/// The person's `AddAccount` is signed later, by `add_admin` again once they registered.
fn confirm_prompt(
    shared: &Shared,
    network: NetworkId,
    signer: &AccountInfo,
    person: String,
) -> Result<AuthorityChallenge, ApiError> {
    let rp = rp_id(shared).to_owned();
    let passkeys = passkeys_for(signer, &rp)?;
    let challenge = b64(&random_bytes()?);
    let expires_at_ms = open_ceremony(
        shared,
        "authority",
        Some(&signer.id),
        &challenge,
        &AuthorityPrompt {
            network: network.to_string(),
            op: String::new(),
            after: After::ConfirmInvite { person },
        },
    )?;
    Ok(AuthorityChallenge {
        change: AuthorityKind::AddAccount,
        public_key: request_options(&rp, &challenge, &passkeys),
        expires_at_ms,
    })
}

/// The signer's passkeys for this site, or 403.
fn passkeys_for(signer: &AccountInfo, rp: &str) -> Result<Vec<Vec<u8>>, ApiError> {
    let passkeys: Vec<Vec<u8>> = signer
        .credentials
        .iter()
        .filter(|passkey| passkey.rp_id == rp)
        .map(|passkey| passkey.credential_id.clone())
        .collect();
    if passkeys.is_empty() {
        return Err(denied("none of your passkeys works on this site"));
    }
    Ok(passkeys)
}

/// Who may see the list: the signed-in person, or the local link's session.
async fn viewer(
    shared: &Shared,
    principal: Option<&Principal>,
) -> Result<(NetworkId, Option<AccountInfo>), ApiError> {
    if principal == Some(&Principal::Local) {
        let (network, _, _) = network(shared).await?;
        return Ok((network, None));
    }
    let (network, person) = signed_in(shared, principal).await?;
    Ok((network, Some(person)))
}

/// `GET /api/people`: everyone in the ledger, then, for owners and admins, the people
/// invited and not yet added.
pub(crate) async fn list(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
) -> Answer {
    let (network, me) = viewer(&shared, principal.as_ref()).await?;
    let me_id = me.as_ref().map(|person| person.id.as_str());
    let mut people: Vec<_> = shared
        .accounts
        .accounts(network)
        .await?
        .iter()
        .map(|account| person_view(account, me_id))
        .collect();
    let sees_pending = me
        .as_ref()
        .is_none_or(|person| person.role.rank() >= PersonRole::Admin.rank());
    if sees_pending {
        let now = shared.clock.now_ms();
        people.extend(
            stored(shared.store.pending(&network.to_string()))?
                .iter()
                .filter(|person| person.credential.is_some() || person.expires_ms > now)
                .map(pending_view),
        );
    }
    Ok(Json(json!(people)))
}

/// 403 unless `actor` may give someone `role`: admins handle members, owners everyone.
fn may_handle(actor: &AccountInfo, role: PersonRole) -> Result<(), ApiError> {
    require(actor, PersonRole::Admin)?;
    if role == PersonRole::Member {
        Ok(())
    } else {
        require(actor, PersonRole::Owner)
    }
}

/// `POST /api/people`: a pending person and their one-time registration link.
pub(crate) async fn invite(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
    input: Result<Json<PersonIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let (network, me) = signed_in(&shared, principal.as_ref()).await?;
    may_handle(&me, input.role)?;
    let token = secret()?;
    let now = shared.clock.now_ms();
    let person = PendingPerson {
        id: format!("p-{}", hex(random_bytes()?.get(..8).unwrap_or_default())),
        token: secret_id(&token),
        network: network.to_string(),
        name: name(&input.name, "a person")?,
        role: input.role.as_str().to_owned(),
        invited_by: me.id,
        created_ms: now,
        expires_ms: now.saturating_add(REGISTER_MS),
        confirmed_ms: None,
        registered_ms: None,
        credential: None,
    };
    stored(shared.store.put_pending(&person))?;
    let link = format!("{}/register/{token}", link_origin(&shared));
    Ok(Json(json!(PersonInvite {
        person: pending_view(&person),
        qr_svg: qr_svg(&link),
        link,
        expires_at_ms: person.expires_ms,
    })))
}

/// The person `id` names in the ledger, or 404.
fn find(accounts: &[AccountInfo], id: &str) -> Result<AccountInfo, ApiError> {
    accounts
        .iter()
        .find(|account| account.id == id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("no such person"))
}

/// The prompt that signs `change`, after the Console's own checks; the ledger checks again.
async fn begin(
    shared: &Shared,
    principal: Option<&Principal>,
    change: AuthorityChangeIn,
) -> Result<AuthorityChallenge, ApiError> {
    let (network, me) = signed_in(shared, principal).await?;
    let accounts = shared.accounts.accounts(network).await?;
    let (change, after) = match change {
        AuthorityChangeIn::AddAdmin { person } => {
            let pending = stored(shared.store.pending(&network.to_string()))?
                .into_iter()
                .find(|one| one.id == person)
                .ok_or_else(|| ApiError::not_found("no such invited person"))?;
            let role = PersonRole::parse(&pending.role).unwrap_or(PersonRole::Member);
            may_handle(&me, role)?;
            let Some(passkey) = pending.credential else {
                return confirm_prompt(shared, network, &me, person);
            };
            (
                AuthorityChange::AddAccount {
                    name: pending.name,
                    role,
                    credential: passkey,
                },
                After::ForgetPending { person },
            )
        }
        AuthorityChangeIn::RemovePerson { person } => {
            let target = find(&accounts, &person)?;
            may_handle(&me, target.role)?;
            (
                AuthorityChange::RemoveAccount {
                    account: target.id.clone(),
                },
                After::EndSessions { account: target.id },
            )
        }
        AuthorityChangeIn::SetRole { person, role } => {
            require(&me, PersonRole::Owner)?;
            let target = find(&accounts, &person)?;
            (
                AuthorityChange::SetRole {
                    account: target.id,
                    role,
                },
                After::Nothing,
            )
        }
        AuthorityChangeIn::AuthorizeConsole { device } => {
            require(&me, PersonRole::Owner)?;
            let device = match device {
                Some(text) => node_id(&text)?,
                None => this_device(shared).await?,
            };
            (AuthorityChange::AuthorizeConsole { device }, After::Nothing)
        }
        AuthorityChangeIn::RevokeConsole { device } => {
            require(&me, PersonRole::Owner)?;
            (
                AuthorityChange::RevokeConsole {
                    device: node_id(&device)?,
                },
                After::Nothing,
            )
        }
    };
    authority_prompt(shared, network, &me, change, after).await
}

async fn this_device(shared: &Shared) -> Result<NodeId, ApiError> {
    current_status(shared)
        .await?
        .node
        .ok_or_else(|| ApiError::not_found("this device has no identity yet"))
}

/// `POST /api/authority/begin`.
pub(crate) async fn authority_begin(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
    input: Result<Json<AuthorityBeginIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    Ok(Json(json!(
        begin(&shared, principal.as_ref(), input.change).await?
    )))
}

/// `POST /api/people/{id}/role`: an owner's prompt to change a role.
pub(crate) async fn role(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
    Path(id): Path<String>,
    input: Result<Json<RoleIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let change = AuthorityChangeIn::SetRole {
        person: id,
        role: input.role,
    };
    Ok(Json(json!(
        begin(&shared, principal.as_ref(), change).await?
    )))
}

/// `POST /api/people/{id}/approve`: the prompt that adds a person who made their passkey.
pub(crate) async fn approve(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
    Path(id): Path<String>,
) -> Answer {
    let change = AuthorityChangeIn::AddAdmin { person: id };
    Ok(Json(json!(
        begin(&shared, principal.as_ref(), change).await?
    )))
}

/// `POST /api/people/{id}/remove`: forgets an invited person at once (an `Outcome`), or
/// answers the prompt that removes a person in the ledger (an `AuthorityChallenge`).
pub(crate) async fn remove(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
    Path(id): Path<String>,
) -> Answer {
    if id.starts_with("p-") {
        let (network, me) = signed_in(&shared, principal.as_ref()).await?;
        let pending = stored(shared.store.pending(&network.to_string()))?
            .into_iter()
            .find(|one| one.id == id)
            .ok_or_else(|| ApiError::not_found("no such invited person"))?;
        may_handle(
            &me,
            PersonRole::parse(&pending.role).unwrap_or(PersonRole::Owner),
        )?;
        stored(shared.store.delete_pending(&id))?;
        return Ok(accepted());
    }
    let change = AuthorityChangeIn::RemovePerson { person: id };
    Ok(Json(json!(
        begin(&shared, principal.as_ref(), change).await?
    )))
}

/// `POST /api/authority/finish`: checks the passkey's answer and has the daemon apply the
/// signed op.
pub(crate) async fn authority_finish(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
    input: Result<Json<CredentialIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let bad = passkey_error(StatusCode::BAD_REQUEST);
    let (_, me) = signed_in(&shared, principal.as_ref()).await?;
    let answer: Authentication = credential(input.credential)?;
    let challenge = answer.challenge().map_err(&bad)?;
    let prompt: AuthorityPrompt = close_ceremony(
        &shared,
        "authority",
        &challenge,
        Some(&me.id),
        StatusCode::BAD_REQUEST,
    )?;
    let assertion = answer.assertion().map_err(&bad)?;
    let passkey = me
        .credentials
        .iter()
        .find(|one| one.credential_id == assertion.credential_id)
        .ok_or_else(|| refused(StatusCode::BAD_REQUEST, "not one of your passkeys"))?;
    with_party(&shared, |party| {
        verify_assertion(
            &assertion,
            &passkey.public_key,
            &passkey.rp_id,
            &challenge,
            Some(party),
        )
    })
    .map_err(&bad)?;
    let network: NetworkId = prompt.network.parse().map_err(|_| ApiError::unexpected())?;
    // A confirmation signs a random challenge, not an op: nothing goes to the ledger.
    if !prompt.op.is_empty() {
        let op_bytes = unb64(&prompt.op).map_err(|_| ApiError::unexpected())?;
        shared
            .accounts
            .submit_authority(
                network,
                op_bytes,
                AccountAssertion {
                    account: me.id.clone(),
                    assertion,
                },
            )
            .await?;
    }
    match prompt.after {
        After::Nothing => {}
        After::ConfirmInvite { person } => {
            stored(shared.store.confirm_pending(&person, shared.clock.now_ms()))?;
        }
        After::ForgetPending { person } => {
            stored(shared.store.delete_pending(&person))?;
        }
        After::EndSessions { account } => {
            stored(shared.store.delete_sessions_of(&account))?;
        }
    }
    Ok(accepted())
}
