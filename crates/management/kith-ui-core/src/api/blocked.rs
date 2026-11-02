//! `POST /api/blocked/:id/allow`: the one rule that allows a blocked connection, previewed
//! before it is signed.
//!
//! The id names the connection itself (network, both devices, protocol and port), so the
//! rule can be signed even after the group left the list; adding a rule that is already
//! there changes nothing, which makes a second allow harmless.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use kith_ipc::{Request, Response};
use kith_types::{NetworkId, NodeId};
use serde_json::json;

use crate::Shared;
use crate::api::{Answer, ApiError, accepted, as_admin, network_id, node_id};
use crate::ipc::{Daemon, Source};
use crate::wire::{AllowIn, AllowPreview};

/// A blocked connection, as its id names it.
#[derive(Debug, PartialEq, Eq)]
struct Connection {
    network: NetworkId,
    from: NodeId,
    to: NodeId,
    protocol: String,
    port: Option<u16>,
}

/// Reads `blocked:<network>:<from>:<to>:<protocol>:<port>`, the port empty when none.
fn parse(id: &str) -> Result<Connection, ApiError> {
    let unknown = || ApiError::bad("not a blocked connection id");
    let mut parts = id.split(':');
    if parts.next() != Some("blocked") {
        return Err(unknown());
    }
    let mut next = || parts.next().ok_or_else(unknown);
    let connection = Connection {
        network: network_id(next()?)?,
        from: node_id(next()?)?,
        to: node_id(next()?)?,
        protocol: next()?.to_owned(),
        port: match next()? {
            "" => None,
            port => Some(port.parse().map_err(|_| unknown())?),
        },
    };
    if parts.next().is_some() {
        return Err(unknown());
    }
    Ok(connection)
}

/// The rule as a sentence, such as `phone may reach nas on TCP 22`.
pub(crate) fn sentence(from: &str, to: &str, protocol: &str, port: Option<u16>) -> String {
    let over = match (protocol, port) {
        ("any", _) => "on any protocol".to_owned(),
        (protocol, Some(port)) => format!("on {} {port}", protocol.to_uppercase()),
        (protocol, None) => format!("on {}", protocol.to_uppercase()),
    };
    format!("{from} may reach {to} {over}")
}

/// The names of both devices: from the blocked list while the group is in it, else from
/// the network's devices and the reports naming this one.
async fn names(daemon: &Daemon, connection: &Connection) -> Result<(String, String), ApiError> {
    let network = connection.network;
    let mut client = daemon.connect().await?;
    if let Ok(Response::Blocked(list)) = client.request(Request::Blocked { network }).await
        && let Some(group) = list.iter().find(|group| {
            group.from == connection.from
                && group.to == connection.to
                && group.protocol == connection.protocol
                && group.port == connection.port
        })
    {
        return Ok((group.from_name.clone(), group.to_name.clone()));
    }
    let Response::Peers(peers) = client.request(Request::Peers { network }).await? else {
        return Err(ApiError::unexpected());
    };
    let reports = match client.request(Request::Topology { network }).await {
        Ok(Response::Topology(reports)) => reports,
        Ok(_) | Err(_) => Vec::new(),
    };
    let name = |node: NodeId| {
        peers
            .iter()
            .find(|peer| peer.node == node)
            .map(|peer| peer.name.clone())
            .or_else(|| {
                reports
                    .iter()
                    .find(|report| report.node == node)
                    .map(|report| report.name.clone())
            })
            .unwrap_or_else(|| node.short().to_string())
    };
    Ok((name(connection.from), name(connection.to)))
}

/// `POST /api/blocked/:id/allow` (tier 1): with `preview`, what would be signed; without,
/// signs it.
pub(crate) async fn allow(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    input: Result<Json<AllowIn>, JsonRejection>,
) -> Answer {
    let Json(input) = input?;
    let daemon = match &shared.source {
        Source::Fixture(sim) => return sim.allow(&id, input.preview),
        Source::Daemon(daemon) => daemon,
    };
    let connection = parse(&id)?;
    if input.preview {
        let (from, to) = names(daemon, &connection).await?;
        let rule = sentence(&from, &to, &connection.protocol, connection.port);
        return Ok(Json(json!(AllowPreview {
            new_paths: vec![rule.clone()],
            rule_sentence: rule,
        })));
    }
    let network = connection.network;
    let request = Request::AllowBlocked {
        network,
        from: connection.from,
        to: connection.to,
        protocol: connection.protocol,
        port: connection.port,
    };
    let Response::BlockedAllowed = as_admin(daemon, network, request).await? else {
        return Err(ApiError::unexpected());
    };
    Ok(accepted())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_read_back_and_rules_read_as_sentences() {
        let network = NetworkId::from_bytes([1; 32]);
        let (from, to) = (NodeId::from_bytes([2; 32]), NodeId::from_bytes([3; 32]));
        let id = format!("blocked:{network}:{from}:{to}:tcp:22");
        assert_eq!(
            parse(&id).ok(),
            Some(Connection {
                network,
                from,
                to,
                protocol: "tcp".to_owned(),
                port: Some(22),
            })
        );
        assert!(
            parse(&format!("blocked:{network}:{from}:{to}:icmp:")).is_ok_and(|c| c.port.is_none())
        );
        assert!(parse(&format!("{id}:9")).is_err());
        assert_eq!(
            sentence("phone", "nas", "tcp", Some(22)),
            "phone may reach nas on TCP 22"
        );
        assert_eq!(
            sentence("phone", "nas", "icmp", None),
            "phone may reach nas on ICMP"
        );
    }
}
