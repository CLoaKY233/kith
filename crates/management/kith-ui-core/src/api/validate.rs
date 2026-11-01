//! `GET /api/validate/name?net=&name=`: whether a device or network name is usable, by the
//! same rules as the command line.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::json;

use crate::Shared;
use crate::api::Answer;
use crate::wire::{NameCheck, NameProblem};

/// The longest DNS label.
const MAX_LABEL: usize = 63;

#[derive(Debug, Deserialize)]
pub(crate) struct NameQuery {
    /// The network the name is for; absent for a new network's name.
    #[expect(
        dead_code,
        reason = "taken names are checked once the daemon can list them"
    )]
    net: Option<String>,
    name: String,
}

/// The name as it would be stored: lowercase, spaces as hyphens, trimmed.
fn normalize(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

fn problem(name: &str) -> Option<NameProblem> {
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

/// `GET /api/validate/name`. Whether the name is taken is not checked yet.
pub(crate) async fn name(
    State(_shared): State<Arc<Shared>>,
    Query(query): Query<NameQuery>,
) -> Answer {
    let name = normalize(&query.name);
    Ok(Json(json!(NameCheck {
        problem: problem(&name),
        suggestion: None,
        name,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_dns_label_rules() {
        assert_eq!(normalize(" Dev Box "), "dev-box");
        assert_eq!(problem("dev-box"), None);
        assert_eq!(problem(""), Some(NameProblem::Empty));
        assert_eq!(problem("-box"), Some(NameProblem::HyphenAtEdge));
        assert_eq!(problem("box_1"), Some(NameProblem::InvalidCharacter));
        assert_eq!(problem(&"a".repeat(64)), Some(NameProblem::TooLong));
    }
}
