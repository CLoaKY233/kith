//! `POST /api/blocked/:id/allow`: the one rule that allows a blocked connection, previewed
//! before it is signed.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError};
use crate::ipc::Source;
use crate::wire::{AllowIn, AllowPreview};

/// `POST /api/blocked/:id/allow`.
pub(crate) async fn allow(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    input: Result<Json<AllowIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    match &shared.source {
        Source::Fixture(fixture) => {
            let group = fixture
                .pending
                .blocked
                .iter()
                .find(|group| group.id == id)
                .ok_or_else(|| ApiError::bad("no such blocked connection"))?;
            if !input.preview {
                return Err(ApiError::fixture());
            }
            let port = group
                .port
                .map(|port| format!(" {port}"))
                .unwrap_or_default();
            let sentence = format!(
                "{} may reach {} on {}{port}",
                group.from_name,
                group.to_name,
                group.protocol.to_uppercase()
            );
            Ok(Json(json!(AllowPreview {
                new_paths: vec![sentence.clone()],
                rule_sentence: sentence,
            })))
        }
        Source::Daemon(_) => Err(ApiError::not_implemented("allowing a blocked connection")),
    }
}
