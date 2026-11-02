//! Admin actions on a network's ledger that the page and CLI share: renames, invites,
//! the policy and relays.

use std::sync::Arc;

use kith_control::NetMap;
use kith_control::records::{Record, Relay};
use kith_ipc::{
    InviteInfo, PolicyInfo, PolicyRuleInfo, RelayInfo, Response, SelectorInfo, TargetInfo,
};
use kith_ledger::OpBody;
use kith_policy::{PortSet, Protocol, Selector};
use kith_types::{NetworkId, NodeId};

use super::Daemon;
use crate::channel::now;
use crate::types::{Action, NodeError};

impl Daemon {
    /// Performs one of this module's actions.
    pub(super) async fn manage(&mut self, action: Action) -> Result<Response, NodeError> {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "execute passes only this module's actions"
        )]
        match action {
            Action::AdvertiseRelay {
                network,
                name,
                address,
                node,
            } => {
                let node = node.unwrap_or_else(|| self.identity.node_id());
                let relay = Relay::new(name, address.clone(), node)
                    .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
                self.write_admin_record(network, &Record::Relay(relay), true)
                    .await?;
                Ok(Response::RelayAdvertised { address })
            }
            Action::RemoveRelay { network, name } => {
                let relay = self
                    .member_map(network)?
                    .relays()
                    .iter()
                    .find(|relay| relay.name() == name)
                    .cloned()
                    .ok_or_else(|| NodeError::NotFound(format!("relay {name}")))?;
                self.write_admin_record(network, &Record::Relay(relay), false)
                    .await?;
                Ok(Response::RelayRemoved)
            }
            Action::RenameDevice {
                network,
                node,
                name,
            } => self.rename_device(network, node, name).await,
            Action::ListInvites { network } => self.list_invites(network),
            Action::RevokeInvite { network, invite } => self.revoke_invite(network, &invite).await,
            Action::GetPolicy { network } => self.policy_info(network),
            Action::ListRelays { network } => self.list_relays(network),
            _ => Err(NodeError::InvalidArgument(
                "not a management action".to_owned(),
            )),
        }
    }

    /// Renames a member device; admins only.
    async fn rename_device(
        &mut self,
        network: NetworkId,
        node: NodeId,
        name: String,
    ) -> Result<Response, NodeError> {
        let member = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?
            .with_ledger(|ledger| ledger.view().is_member(&node))?;
        if !member {
            return Err(NodeError::NotFound(format!(
                "{node} is not in this network"
            )));
        }
        self.admin_only(network, "rename devices")?;
        let identity = Arc::clone(&self.identity);
        self.runtime_mut(network)?.append(
            &identity,
            OpBody::RenameDevice { device: node, name },
            now(),
        )?;
        self.commit(network).await?;
        Ok(Response::DeviceRenamed)
    }

    /// The invites that can still admit devices and have not expired.
    fn list_invites(&self, network: NetworkId) -> Result<Response, NodeError> {
        self.member_map(network)?;
        let at = now();
        let invites = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?
            .with_ledger(|ledger| {
                ledger
                    .view()
                    .invites()
                    .iter()
                    .filter(|(_, invite)| invite.expires >= at)
                    .map(|(key, invite)| InviteInfo {
                        id: hex(key),
                        expires_at_ms: invite.expires.as_millis(),
                        uses_left: (invite.max_uses != u16::MAX)
                            .then(|| u32::from(invite.max_uses.saturating_sub(invite.uses))),
                        created_at_ms: invite.created.as_millis(),
                    })
                    .collect()
            })?;
        Ok(Response::Invites(invites))
    }

    /// Revokes the invite whose key is `id` in hex; admins only.
    async fn revoke_invite(&mut self, network: NetworkId, id: &str) -> Result<Response, NodeError> {
        self.member_map(network)?;
        let wanted = id.trim().to_ascii_lowercase();
        let key = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?
            .with_ledger(|ledger| {
                ledger
                    .view()
                    .invites()
                    .iter()
                    .find(|(bytes, _)| hex(bytes.as_slice()) == wanted)
                    .map(|(_, invite)| invite.key)
            })?
            .ok_or_else(|| NodeError::NotFound(format!("invite {wanted}")))?;
        self.admin_only(network, "revoke invites")?;
        let identity = Arc::clone(&self.identity);
        self.runtime_mut(network)?.append(
            &identity,
            OpBody::RevokeInvite { invite_key: key },
            now(),
        )?;
        self.commit(network).await?;
        Ok(Response::InviteRevoked)
    }

    /// The policy in force, as TOML and structured.
    fn policy_info(&self, network: NetworkId) -> Result<Response, NodeError> {
        let map = self.member_map(network)?;
        let local = self.identity.node_id();
        let policy = self
            .networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?
            .policy()?;
        let toml = toml::to_string(&policy)
            .map_err(|error| NodeError::InvalidArgument(format!("policy: {error}")))?;
        let rules = policy
            .rules()
            .iter()
            .map(|rule| PolicyRuleInfo {
                sources: rule
                    .sources()
                    .iter()
                    .map(|selector| selector_info(map, local, selector))
                    .collect(),
                destinations: rule
                    .destinations()
                    .iter()
                    .map(|target| TargetInfo {
                        selector: selector_info(map, local, target.selector()),
                        protocol: protocol_name(target.protocol()).to_owned(),
                        ports: ports(target.ports()),
                    })
                    .collect(),
            })
            .collect();
        Ok(Response::Policy(PolicyInfo { toml, rules }))
    }

    /// The relays advertised in a network.
    fn list_relays(&self, network: NetworkId) -> Result<Response, NodeError> {
        let relays = self
            .member_map(network)?
            .relays()
            .iter()
            .map(|relay| RelayInfo {
                name: relay.name().to_owned(),
                address: relay.address().to_owned(),
                node: relay.node(),
            })
            .collect();
        Ok(Response::Relays(relays))
    }

    fn admin_only(&self, network: NetworkId, what: &str) -> Result<(), NodeError> {
        if self.is_admin(network)? {
            Ok(())
        } else {
            Err(NodeError::InvalidArgument(format!(
                "only an admin of this network can {what}"
            )))
        }
    }
}

/// The name a network gives `node`, if it knows the device.
fn label(map: &NetMap, local: NodeId, node: &NodeId) -> Option<String> {
    map.peers()
        .get(node)
        .or_else(|| map.clients().get(node).map(|client| &client.peer))
        .or_else(|| map.local_device().filter(|_| local == *node))
        .map(|peer| peer.label.clone())
}

fn selector_info(map: &NetMap, local: NodeId, selector: &Selector) -> SelectorInfo {
    match selector {
        Selector::Any => SelectorInfo {
            kind: "any".to_owned(),
            value: None,
            name: None,
        },
        Selector::Device(node) => SelectorInfo {
            kind: "device".to_owned(),
            value: Some(node.to_string()),
            name: label(map, local, node),
        },
        Selector::Tag(tag) => SelectorInfo {
            kind: "tag".to_owned(),
            value: Some(tag.to_string()),
            name: None,
        },
    }
}

const fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Any => "any",
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
        Protocol::Icmp => "icmp",
    }
}

fn ports(set: &PortSet) -> Vec<String> {
    match set {
        PortSet::All => Vec::new(),
        PortSet::Only(ranges) => ranges
            .as_slice()
            .iter()
            .map(|range| {
                if range.start() == range.end() {
                    range.start().to_string()
                } else {
                    format!("{}-{}", range.start(), range.end())
                }
            })
            .collect(),
    }
}

/// Lowercase hex, the form invite ids take.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
