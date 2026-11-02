//! `/api/networks/:net/policy`: who may reach whom, as TOML and as rules.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};

use crate::api::{Answer, ApiError, accepted, as_admin, network_id};
use crate::ipc::Source;
use crate::wire::PolicyIn;
use crate::{Shared, json};

/// `GET /api/networks/:net/policy`: `{toml, rules}`.
pub(crate) async fn get(State(shared): State<Arc<Shared>>, Path(net): Path<String>) -> Answer {
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.policy(&net),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let Response::Policy(policy) = daemon.ask(Request::GetPolicy { network }).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(Json(json::to_value(&policy)?))
}

/// `POST /api/networks/:net/policy` (tier 2): replaces the policy; TOML that does not
/// parse answers 400 with the parser's line and column.
pub(crate) async fn set(
    State(shared): State<Arc<Shared>>,
    Path(net): Path<String>,
    input: Result<Json<PolicyIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.set_policy(&net, &input),
        Source::Daemon(daemon) => daemon,
    };
    let network = network_id(&net)?;
    let request = Request::SetPolicy {
        network,
        policy_toml: input.toml,
    };
    let Response::PolicySet = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}
