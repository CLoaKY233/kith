//! `GET /api/validate/name?net=&name=`: whether a device or network name is usable.
//!
//! The rules are the ones every device applies when it turns a name into its DNS label:
//! lowercase letters, digits and single hyphens, at most 63 characters. A name is taken
//! when another device in the network, or another network of this device, has it.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use kith_ipc::{Request, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;
use crate::api::status::local;
use crate::api::{Answer, ApiError};
use crate::ipc::{Daemon, Source};
use crate::wire::{Fixture, NameCheck, NameProblem};

/// The longest DNS label.
const MAX_LABEL: usize = 63;
/// The highest number tried after a taken name, as in `laptop-2`.
const MAX_SUFFIX: u32 = 99;

#[derive(Debug, Deserialize)]
pub(crate) struct NameQuery {
    /// The network the name is for, by id or name; empty or absent for a new network.
    net: Option<String>,
    name: String,
}

/// The name as it would be stored: lowercase, trimmed, each run of spaces and hyphens
/// one hyphen, as the daemon turns a name into its label.
pub(crate) fn normalize(name: &str) -> String {
    let mut label = String::with_capacity(name.len());
    for ch in name.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_whitespace() || ch == '-' {
            if !label.ends_with('-') {
                label.push('-');
            }
        } else {
            label.push(ch);
        }
    }
    label
}

pub(crate) fn problem(name: &str) -> Option<NameProblem> {
    if name.is_empty() {
        Some(NameProblem::Empty)
    } else if name.len() > MAX_LABEL {
        Some(NameProblem::TooLong)
    } else if !name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Some(NameProblem::InvalidCharacter)
    } else if name.starts_with('-') || name.ends_with('-') {
        Some(NameProblem::HyphenAtEdge)
    } else {
        None
    }
}

/// The first of `name-2`, `name-3`, … that is free and still a valid label.
fn suggest(name: &str, taken: &BTreeSet<String>) -> Option<String> {
    (2..=MAX_SUFFIX)
        .map(|number| {
            let suffix = format!("-{number}");
            let stem = name
                .get(..MAX_LABEL.saturating_sub(suffix.len()).min(name.len()))
                .unwrap_or(name)
                .trim_end_matches('-');
            format!("{stem}{suffix}")
        })
        .find(|candidate| !taken.contains(candidate))
}

/// The names in use where `name` would go, from a recording; 404 for a network it does
/// not hold.
fn recorded(fixture: &Fixture, net: Option<&str>) -> Result<BTreeSet<String>, ApiError> {
    let networks = fixture
        .status
        .status
        .get("networks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let field =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let Some(net) = net else {
        return Ok(networks
            .iter()
            .filter_map(|network| field(network, "name"))
            .collect());
    };
    let Some(id) = networks
        .iter()
        .find(|network| {
            field(network, "id").as_deref() == Some(net)
                || field(network, "name").as_deref() == Some(net)
        })
        .and_then(|network| field(network, "id"))
    else {
        return Err(unknown_network());
    };
    let devices = fixture
        .peers
        .get(&id)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let me = fixture
        .status
        .memberships
        .iter()
        .filter(|membership| membership.network == id)
        .map(|membership| membership.name.clone());
    Ok(devices
        .iter()
        .filter_map(|peer| field(peer, "name"))
        .chain(me)
        .collect())
}

fn unknown_network() -> ApiError {
    ApiError::not_found("this device is not in that network")
}

/// The names in use where `name` would go, from the daemon.
async fn in_use(daemon: &Daemon, net: Option<&str>) -> Result<BTreeSet<String>, ApiError> {
    let mut client = daemon.connect().await?;
    let Response::Status(status) = client.request(Request::Status).await? else {
        return Err(ApiError::unexpected());
    };
    let Some(net) = net else {
        return Ok(status
            .networks
            .into_iter()
            .map(|summary| summary.name)
            .collect());
    };
    let Some(summary) = status
        .networks
        .iter()
        .find(|summary| summary.id.to_string() == net || summary.name == net)
    else {
        return Err(unknown_network());
    };
    let Response::Peers(peers) = client
        .request(Request::Peers {
            network: summary.id,
        })
        .await?
    else {
        return Err(ApiError::unexpected());
    };
    let mut names: BTreeSet<String> = peers.into_iter().map(|peer| peer.name).collect();
    if let Some(node) = status.node {
        names.insert(local(&mut client, summary, node).await?.0.name);
    }
    Ok(names)
}

/// `GET /api/validate/name`.
pub(crate) async fn name(
    State(shared): State<Arc<Shared>>,
    query: Result<Query<NameQuery>, QueryRejection>,
) -> Answer {
    let Query(query) = query.map_err(|rejection| ApiError::bad(rejection.body_text()))?;
    let name = normalize(&query.name);
    if let Some(problem) = problem(&name) {
        return Ok(Json(json!(NameCheck {
            name,
            problem: Some(problem),
            suggestion: None,
        })));
    }
    let net = query.net.as_deref().filter(|net| !net.is_empty());
    let taken = match &shared.source {
        Source::Fixture(sim) => recorded(&sim.read(), net)?,
        Source::Daemon(daemon) => in_use(daemon, net).await?,
    };
    let check = if taken.contains(&name) {
        NameCheck {
            suggestion: suggest(&name, &taken),
            problem: Some(NameProblem::Taken),
            name,
        }
    } else {
        NameCheck {
            name,
            problem: None,
            suggestion: None,
        }
    };
    Ok(Json(json!(check)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_dns_label_rules() {
        assert_eq!(normalize(" Dev Box "), "dev-box");
        assert_eq!(normalize("my -- Laptop"), "my-laptop");
        assert_eq!(problem("dev-box"), None);
        assert_eq!(problem(""), Some(NameProblem::Empty));
        assert_eq!(problem("-box"), Some(NameProblem::HyphenAtEdge));
        assert_eq!(problem("box_1"), Some(NameProblem::InvalidCharacter));
        assert_eq!(problem(&"a".repeat(64)), Some(NameProblem::TooLong));
    }

    #[test]
    fn a_taken_name_suggests_the_next_free_number() {
        let taken: BTreeSet<String> = ["laptop", "laptop-2"].map(str::to_owned).into();
        assert_eq!(suggest("laptop", &taken).as_deref(), Some("laptop-3"));
        let long = "a".repeat(MAX_LABEL);
        let suggested = suggest(&long, &BTreeSet::new()).unwrap_or_default();
        assert_eq!(suggested.len(), MAX_LABEL);
        assert!(suggested.ends_with("-2"));
    }
}
