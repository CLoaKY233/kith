//! Signing in and out with passkeys, setting the Console up, and an invited person
//! making their passkey.
//!
//! Setup follows `docs/plans/console.md`: the setup link's page names the network and
//! the owner and makes the owner's passkey; this device signs the first `AddAccount`
//! (owner); the owner's passkey then signs `AuthorizeConsole` for this device through
//! `/api/authority/finish`; `/api/setup/recovery` shows the recovery words once.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use kith_ipc::{Request, Response as Reply};
use kith_types::NetworkId;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Shared;
use crate::accounts::AuthorityChange;
use crate::api::people::authority_prompt;
use crate::api::{Answer, ApiError};
use crate::console::{
    Mode, Principal, choose_network, close_ceremony, console_network, credential, link_refusal,
    mark_setup_done, network, network_ref, open_ceremony, passkey_error, pending_view, person_view,
    random_bytes, refused, require, signed_in, start_session, stored, with_party,
};
use crate::ipc::Source;
use crate::session::Caller;
use crate::store::secret_id;
use crate::webauthn::{
    Authentication, Registration, b64, creation_options, request_options, verify_assertion,
    verify_registration,
};
use crate::wire::{
    CeremonyOptions, ConsoleMode, CreateNetworkIn, CredentialIn, MeView, PersonRole, RecoveryWords,
    RegisterInfo, SetupBeginIn, SetupFinished, SetupInfo, TokenCredentialIn, TokenIn,
};

/// The longest name a person or network may have, in bytes.
const MAX_NAME: usize = 64;

/// What the setup prompt needs to finish.
#[derive(Debug, Serialize, Deserialize)]
struct SetupPrompt {
    token: String,
    name: String,
    network_name: Option<String>,
}

/// What a registration prompt needs to finish.
#[derive(Debug, Serialize, Deserialize)]
struct RegisterPrompt {
    person: String,
}

/// A name as given, trimmed, or 400.
pub(crate) fn name(text: &str, what: &str) -> Result<String, ApiError> {
    let name = text.trim();
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(ApiError::bad(format!(
            "{what} needs 1 to {MAX_NAME} characters"
        )));
    }
    Ok(name.to_owned())
}

/// A fresh challenge, as base64url.
fn challenge() -> Result<String, ApiError> {
    Ok(b64(&random_bytes()?))
}

/// The answer with a session cookie set.
fn with_cookie(body: impl Serialize, cookie: axum::http::HeaderValue) -> Response {
    let mut response = Json(json!(body)).into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    response
}

/// `GET /api/me`.
pub(crate) async fn me(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
) -> Answer {
    let mode = match shared.mode {
        Mode::Local => ConsoleMode::Local,
        Mode::Public { .. } => ConsoleMode::Public,
    };
    let person = match &principal {
        Some(Principal::Person { account, .. }) => {
            let (_, person) = signed_in(&shared, principal.as_ref()).await?;
            Some(person_view(&person, Some(account)))
        }
        Some(Principal::Local) | None => None,
    };
    let network = console_network(&shared)
        .await?
        .map(|(id, name, _)| network_ref(id, &name));
    Ok(Json(json!(MeView {
        mode,
        person,
        network,
    })))
}

/// `POST /api/logout`: ends this browser's session.
pub(crate) async fn logout(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
) -> Response {
    for secret in shared.sessions.secrets_in(request.headers()) {
        let _ended = shared.store.delete_session(&secret_id(secret));
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(cookie) = axum::http::HeaderValue::from_str(&shared.sessions.clear_cookie()) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

/// `GET /api/setup/{token}`.
pub(crate) async fn setup_info(
    State(shared): State<Arc<Shared>>,
    Path(token): Path<String>,
) -> Answer {
    let expires_at_ms = link_refusal(stored(
        shared
            .store
            .check_setup_token(&secret_id(&token), shared.clock.now_ms()),
    )?)?;
    let network = console_network(&shared)
        .await?
        .map(|(id, name, _)| network_ref(id, &name));
    Ok(Json(json!(SetupInfo {
        network,
        expires_at_ms,
    })))
}

/// `POST /api/setup/begin`: the owner's passkey prompt.
pub(crate) async fn setup_begin(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<SetupBeginIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let token = secret_id(&input.token);
    link_refusal(stored(
        shared
            .store
            .check_setup_token(&token, shared.clock.now_ms()),
    )?)?;
    let owner = name(&input.name, "your name")?;
    let network_name = match console_network(&shared).await? {
        Some(_) => None,
        None => Some(name(
            input.network_name.as_deref().unwrap_or_default(),
            "the network's name",
        )?),
    };
    let challenge = challenge()?;
    let user = random_bytes()?;
    open_ceremony(
        &shared,
        "setup",
        None,
        &challenge,
        &SetupPrompt {
            token,
            name: owner.clone(),
            network_name,
        },
    )?;
    let options = creation_options(
        crate::console::rp_id(&shared),
        user.get(..16).unwrap_or_default(),
        &owner,
        &challenge,
        &[],
    );
    Ok(Json(json!(CeremonyOptions {
        public_key: options
    })))
}

/// `POST /api/setup/finish`: adds the owner, signs them in, and asks their passkey to
/// authorize this device.
pub(crate) async fn setup_finish(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<TokenCredentialIn>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(input) = input?;
    let registration: Registration = credential(input.credential)?;
    let challenge = registration
        .challenge()
        .map_err(passkey_error(StatusCode::BAD_REQUEST))?;
    let prompt: SetupPrompt =
        close_ceremony(&shared, "setup", &challenge, None, StatusCode::BAD_REQUEST)?;
    if prompt.token != secret_id(&input.token) {
        return Err(refused(
            StatusCode::BAD_REQUEST,
            "this prompt is for another link",
        ));
    }
    let passkey = with_party(&shared, |party| {
        verify_registration(&registration, &challenge, party)
    })
    .map_err(passkey_error(StatusCode::BAD_REQUEST))?;
    link_refusal(stored(
        shared
            .store
            .check_setup_token(&prompt.token, shared.clock.now_ms()),
    )?)?;
    let network = if let Some((id, _, _)) = console_network(&shared).await? {
        id
    } else {
        let network_name = prompt
            .network_name
            .ok_or_else(|| ApiError::bad("name the network"))?;
        create_network(&shared, &network_name).await?
    };
    choose_network(&shared, network)?;
    // The recovery key can only be made while the network has no people: before the
    // owner. A network that already has one keeps it; setup goes on without words.
    // Setup never finishes without recovery words: if the network has none yet and making
    // them fails, setup stops here, before the owner exists.
    let has_people = !shared.accounts.accounts(network).await?.is_empty();
    let words =
        if has_people {
            None
        } else {
            Some(shared.accounts.recovery_key(network).await.map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::wire::ApiErrorCode::Internal,
                format!(
                    "setup stopped before adding you, because the recovery words could not \
                     be made: {}. Check that kith is running on the server and that the \
                     Console may change this network, then open the setup link again",
                    error.message()
                ),
            )
        })?)
        };
    let owner = shared
        .accounts
        .add_account(network, prompt.name, PersonRole::Owner, passkey)
        .await?;
    if let Some(words) = words {
        crate::api::lock(&shared.api.recovery).insert(owner.id.clone(), words);
    }
    // The link is burned last, so a failure above leaves it good for another try.
    link_refusal(stored(
        shared
            .store
            .use_setup_token(&prompt.token, shared.clock.now_ms()),
    )?)?;
    mark_setup_done(&shared)?;
    let (_, _, device) = crate::console::network(&shared).await?;
    let device = device.ok_or_else(ApiError::unexpected)?;
    let authority = authority_prompt(
        &shared,
        network,
        &owner,
        AuthorityChange::AuthorizeConsole { device },
        crate::api::people::After::Nothing,
    )
    .await?;
    let cookie = start_session(&shared, &owner.id)?;
    Ok(with_cookie(
        SetupFinished {
            person: person_view(&owner, Some(&owner.id)),
            authority,
        },
        cookie,
    ))
}

/// Makes the Console's network, with this device as its first admin.
async fn create_network(shared: &Shared, name: &str) -> Result<NetworkId, ApiError> {
    let device_name = match shared.api.host.name.trim() {
        "" => "console".to_owned(),
        host => host.to_owned(),
    };
    match &shared.source {
        Source::Fixture(sim) => {
            let _created = sim.create_network(&CreateNetworkIn {
                name: name.to_owned(),
                device_name,
            })?;
            let status = sim.status_now()?;
            status
                .networks
                .iter()
                .rev()
                .find(|network| network.name == name)
                .map(|network| network.id)
                .ok_or_else(ApiError::unexpected)
        }
        Source::Daemon(daemon) => {
            let request = Request::CreateNetwork {
                name: name.to_owned(),
                device_name,
            };
            let Reply::NetworkCreated { network } = daemon.ask(request).await? else {
                return Err(ApiError::unexpected());
            };
            Ok(network)
        }
    }
}

/// `POST /api/setup/recovery`: the recovery words setup made, once, to the owner it added.
pub(crate) async fn recovery(
    State(shared): State<Arc<Shared>>,
    Extension(Caller(principal)): Extension<Caller>,
) -> Answer {
    let (_, person) = signed_in(&shared, principal.as_ref()).await?;
    require(&person, PersonRole::Owner)?;
    let words = crate::api::lock(&shared.api.recovery)
        .remove(&person.id)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                crate::wire::ApiErrorCode::Conflict,
                "the recovery words were shown already, or were never made here",
            )
        })?;
    Ok(Json(json!(RecoveryWords { words })))
}

/// `POST /api/login/begin`: a prompt for any passkey of this site.
pub(crate) async fn login_begin(State(shared): State<Arc<Shared>>) -> Answer {
    let challenge = challenge()?;
    open_ceremony(&shared, "login", None, &challenge, &())?;
    Ok(Json(json!(CeremonyOptions {
        public_key: request_options(crate::console::rp_id(&shared), &challenge, &[])
    })))
}

/// `POST /api/login/finish`: a session for the person whose passkey answered.
pub(crate) async fn login_finish(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<CredentialIn>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(input) = input?;
    let unauthorized = passkey_error(StatusCode::UNAUTHORIZED);
    let answer: Authentication = credential(input.credential)?;
    let challenge = answer.challenge().map_err(&unauthorized)?;
    close_ceremony::<()>(&shared, "login", &challenge, None, StatusCode::UNAUTHORIZED)?;
    let assertion = answer.assertion().map_err(&unauthorized)?;
    let (network, _, _) = network(&shared).await?;
    let rp_id = crate::console::rp_id(&shared).to_owned();
    let accounts = shared.accounts.accounts(network).await?;
    let (person, passkey) = accounts
        .iter()
        .find_map(|account| {
            account
                .credentials
                .iter()
                .find(|one| one.credential_id == assertion.credential_id && one.rp_id == rp_id)
                .map(|passkey| (account, passkey))
        })
        .ok_or_else(|| {
            refused(
                StatusCode::UNAUTHORIZED,
                "this passkey belongs to no one in this network",
            )
        })?;
    with_party(&shared, |party| {
        verify_assertion(
            &assertion,
            &passkey.public_key,
            &rp_id,
            &challenge,
            Some(party),
        )
    })
    .map_err(&unauthorized)?;
    let cookie = start_session(&shared, &person.id)?;
    let mode = match shared.mode {
        Mode::Local => ConsoleMode::Local,
        Mode::Public { .. } => ConsoleMode::Public,
    };
    let network_name = console_network(&shared)
        .await?
        .map(|(id, name, _)| network_ref(id, &name));
    Ok(with_cookie(
        MeView {
            mode,
            person: Some(person_view(person, Some(&person.id))),
            network: network_name,
        },
        cookie,
    ))
}

/// The invited person a registration link names: 404 for none, 410 once it lapsed
/// without a passkey.
fn invited(shared: &Shared, token: &str) -> Result<crate::store::PendingPerson, ApiError> {
    let person = stored(shared.store.pending_by_token(&secret_id(token)))?
        .ok_or_else(|| ApiError::not_found("no such link"))?;
    if person.credential.is_none() && person.expires_ms <= shared.clock.now_ms() {
        return link_refusal(crate::store::Redeem::Expired);
    }
    Ok(person)
}

/// 403 for an owner's or admin's invite their inviter has not confirmed with a passkey.
fn confirmed(person: &crate::store::PendingPerson) -> Result<(), ApiError> {
    if person.role != PersonRole::Member.as_str() && person.confirmed_ms.is_none() {
        return Err(crate::console::denied(
            "this invite waits for its inviter to confirm it with a passkey",
        ));
    }
    Ok(())
}

/// `GET /api/register/{token}`.
pub(crate) async fn register_info(
    State(shared): State<Arc<Shared>>,
    Path(token): Path<String>,
) -> Answer {
    let person = invited(&shared, &token)?;
    let network: NetworkId = person.network.parse().map_err(|_| ApiError::unexpected())?;
    let (_, network_name, _) = crate::console::network(&shared).await?;
    let accounts = shared.accounts.accounts(network).await?;
    let invited_by = accounts
        .iter()
        .find(|account| account.id == person.invited_by)
        .map_or_else(|| "an admin".to_owned(), |account| account.name.clone());
    Ok(Json(json!(RegisterInfo {
        name: person.name.clone(),
        role: PersonRole::parse(&person.role).unwrap_or(PersonRole::Member),
        network: network_ref(network, &network_name),
        invited_by,
        expires_at_ms: person.expires_ms,
        registered: person.credential.is_some(),
    })))
}

/// `POST /api/register/begin`: an invited person's passkey prompt.
pub(crate) async fn register_begin(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<TokenIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let person = invited(&shared, &input.token)?;
    if person.credential.is_some() {
        return link_refusal(crate::store::Redeem::Used);
    }
    confirmed(&person)?;
    let challenge = challenge()?;
    let user = random_bytes()?;
    open_ceremony(
        &shared,
        "register",
        None,
        &challenge,
        &RegisterPrompt {
            person: person.id.clone(),
        },
    )?;
    Ok(Json(json!(CeremonyOptions {
        public_key: creation_options(
            crate::console::rp_id(&shared),
            user.get(..16).unwrap_or_default(),
            &person.name,
            &challenge,
            &[],
        )
    })))
}

/// `POST /api/register/finish`: keeps the passkey until an owner or admin approves the
/// person.
pub(crate) async fn register_finish(
    State(shared): State<Arc<Shared>>,
    input: Result<Json<TokenCredentialIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let registration: Registration = credential(input.credential)?;
    let challenge = registration
        .challenge()
        .map_err(passkey_error(StatusCode::BAD_REQUEST))?;
    let prompt: RegisterPrompt = close_ceremony(
        &shared,
        "register",
        &challenge,
        None,
        StatusCode::BAD_REQUEST,
    )?;
    let person = invited(&shared, &input.token)?;
    confirmed(&person)?;
    if person.id != prompt.person {
        return Err(refused(
            StatusCode::BAD_REQUEST,
            "this prompt is for another link",
        ));
    }
    let passkey = with_party(&shared, |party| {
        verify_registration(&registration, &challenge, party)
    })
    .map_err(passkey_error(StatusCode::BAD_REQUEST))?;
    link_refusal(stored(shared.store.register_pending(
        &person.id,
        &passkey,
        shared.clock.now_ms(),
    ))?)?;
    let registered = crate::store::PendingPerson {
        registered_ms: Some(shared.clock.now_ms()),
        credential: Some(passkey),
        ..person
    };
    Ok(Json(json!(pending_view(&registered))))
}
