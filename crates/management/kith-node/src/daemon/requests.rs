//! Join requests: asking with a request code, and an admin approving or denying.

use std::sync::Arc;

use kith_channel::ChannelError;
use kith_control::NetMap;
use kith_identity::RequestCode;
use kith_ipc::{EntryKind, PendingJoin, Response};
use kith_ledger::{AdmissionRequest, OpBody};
use kith_types::{NetworkId, NodeId};

use super::Daemon;
use crate::admissions::{REFRESH_AFTER, Refusal};
use crate::channel::{lock_admissions, now};
use crate::types::NodeError;

impl Daemon {
    pub(super) fn request_code(&self, network: NetworkId) -> Result<Response, NodeError> {
        let name = self.member_map(network)?.name().to_owned();
        let code = RequestCode::new(network, name, self.bootstrap())?;
        Ok(Response::RequestCode {
            code: code.to_link()?,
        })
    }

    /// Asks a member in `code` to hold this device's request, or learns the answer. The
    /// same signed request is reused for [`REFRESH_AFTER`], so admins see one request, and
    /// then re-signed, so it outlives a member whose clock runs ahead.
    pub(super) async fn request_join(
        &mut self,
        code: &RequestCode,
        device_name: &str,
    ) -> Result<Response, NodeError> {
        let network = code.network();
        if self.networks.get(&network).is_some() {
            return Err(NodeError::InvalidArgument(format!(
                "already joined {network}"
            )));
        }
        let at = now();
        let reusable = self.asks.get(&network).filter(|asked| {
            asked.name == device_name
                && asked.requested_at <= at
                && at.saturating_duration_since(asked.requested_at) < REFRESH_AFTER
        });
        let request = if let Some(asked) = reusable {
            asked.clone()
        } else {
            let created =
                AdmissionRequest::create(network, self.identity.signing_key(), device_name, at)?;
            self.asks.insert(network, created.clone());
            created
        };
        match self.channel.ask(code.bootstrap(), &request).await {
            Ok(None) => Ok(Response::JoinPending {
                network,
                network_name: code.network_name().to_owned(),
            }),
            Ok(Some(ledger)) => {
                self.asks.remove(&network);
                self.finish_join(ledger, network, code.bootstrap(), &request.name)
                    .await
            }
            Err(NodeError::Channel(ChannelError::JoinRejected(reason)))
                if reason == Refusal::Denied.to_string() =>
            {
                self.asks.remove(&network);
                Ok(Response::JoinDenied { network })
            }
            Err(NodeError::Channel(ChannelError::JoinRejected(reason)))
                if reason == Refusal::Expired.to_string() =>
            {
                self.asks.remove(&network);
                Err(NodeError::Channel(ChannelError::JoinRejected(reason)))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn pending_joins(&self) -> Response {
        let at = now();
        let mut admissions = lock_admissions(self.channel.admissions());
        let mut list = Vec::new();
        for (network, map) in self.networks.maps().iter() {
            if !map.is_member() {
                continue;
            }
            for held in admissions.pending(*network, at) {
                if map.peers().contains_key(&held.request.device) {
                    continue;
                }
                list.push(PendingJoin {
                    network: *network,
                    node: held.request.device,
                    name: held.request.name,
                    from: held.from.map(|addr| addr.to_string()),
                    age_secs: at
                        .saturating_duration_since(held.request.requested_at)
                        .as_secs(),
                });
            }
        }
        Response::PendingJoins(list)
    }

    /// Adds the device with an ordinary signed `AddDevice` op under the name it asked for.
    pub(super) async fn approve_join(
        &mut self,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Response, NodeError> {
        self.require_admin(network)?;
        let member = self
            .networks
            .maps()
            .get(&network)
            .and_then(|map| map.peers().get(&node))
            .map(|peer| peer.label.clone());
        if let Some(device) = member {
            lock_admissions(self.channel.admissions()).take(network, node);
            return Ok(Response::JoinApproved { device });
        }
        let at = now();
        let held = lock_admissions(self.channel.admissions())
            .pending(network, at)
            .into_iter()
            .find(|held| held.request.device == node)
            .ok_or_else(|| NodeError::NotFound(format!("no pending request from {node}")))?;
        let identity = Arc::clone(&self.identity);
        let author = identity.node_id();
        let body = OpBody::AddDevice {
            device: node,
            name: held.request.name.clone(),
            tags: Vec::new(),
        };
        self.runtime_mut(network)?.with_ledger(|ledger| {
            let op = ledger
                .draft(author, body, at)
                .sign(identity.signing_key())?;
            ledger.insert(op).map(|_| ())
        })??;
        lock_admissions(self.channel.admissions()).take(network, node);
        self.commit(network).await?;
        self.gossip_admissions(network)?;
        let device = self
            .networks
            .maps()
            .get(&network)
            .and_then(|map| map.peers().get(&node))
            .map_or(held.request.name, |peer| peer.label.clone());
        Ok(Response::JoinApproved { device })
    }

    /// Denies the request here and gossips the denial, so it leaves other admins' lists.
    pub(super) fn deny_join(
        &mut self,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Response, NodeError> {
        self.require_admin(network)?;
        lock_admissions(self.channel.admissions())
            .deny(network, node, now())
            .ok_or_else(|| NodeError::NotFound(format!("no pending request from {node}")))?;
        self.gossip_admissions(network)?;
        Ok(Response::JoinRequestDenied)
    }

    /// Journals newly arrived requests and passes the current set on to other members.
    pub(super) fn on_admissions(
        &mut self,
        network: NetworkId,
        new: Vec<(NodeId, String)>,
    ) -> Result<(), NodeError> {
        if !self
            .networks
            .maps()
            .get(&network)
            .is_some_and(NetMap::is_member)
        {
            return Ok(());
        }
        for (node, name) in new {
            self.journal.push(EntryKind::JoinRequested {
                network,
                node,
                name,
            });
        }
        self.gossip_admissions(network)
    }

    fn gossip_admissions(&self, network: NetworkId) -> Result<(), NodeError> {
        let (requests, denied) =
            lock_admissions(self.channel.admissions()).snapshot(network, now());
        self.channel.gossip_admissions(network, requests, denied)
    }

    fn require_admin(&self, network: NetworkId) -> Result<(), NodeError> {
        self.member_map(network)?;
        if self.is_admin(network)? {
            Ok(())
        } else {
            Err(NodeError::InvalidArgument(String::from(
                "only an admin of this network can approve or deny join requests",
            )))
        }
    }
}
