//! `kith-ui --fixture` as a simulator: the recorded network is a working copy that every
//! change edits in memory, and each change writes the journal entry a daemon would, so
//! the page sees its own actions on the event stream.
//!
//! Nothing here reaches a daemon or the network: invites, phone configs and imports are
//! made up, and the boot payload marks the whole session as a fixture.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use axum::Json;
use axum::http::StatusCode;
use kith_identity::{InviteToken, RequestCode};
use kith_ipc::{
    EntryKind, ImportedInfo, PeerInfo, PolicyInfo, PolicyRuleInfo, SelectorInfo, Status, TargetInfo,
};
use kith_policy::{PortSet, Protocol, Rule, Selector, Target};
use kith_types::{NetworkId, NodeId, Timestamp};
use serde_json::{Value, json};

use crate::api::codes::{check_code, key_fpr};
use crate::api::status::suffix;
use crate::api::validate::{normalize, problem};
use crate::api::{Answer, ApiError, HeldPhone, accepted, network_id, node_id, qr_svg, settled};
use crate::stream::Live;
use crate::wire::{
    AllowPreview, ApiErrorCode, CreateNetworkIn, ExitIn, ExitView, Fixture, ImportIn, Imported,
    Invite, InviteIn, JoinIn, JoinRequestState, Membership, OpenInvite, PhoneCreated, PhoneIn,
    PolicyIn, RecordIn, RecordType, RecordView, RelayIn, RelayView, Resolution, Role, ServerView,
};
use crate::{json, now_ms};

/// The recording, changed by what the page does.
#[derive(Debug)]
pub(crate) struct Sim {
    world: Mutex<Fixture>,
}

/// What `allow` needs of a blocked connection.
struct Blocked {
    from: NodeId,
    to: NodeId,
    protocol: String,
    port: Option<u16>,
}

impl Sim {
    pub(crate) fn new(fixture: Fixture) -> Self {
        Self {
            world: Mutex::new(fixture),
        }
    }

    /// The recording as it stands now.
    /// The simulated daemon's status.
    pub(crate) fn status_now(&self) -> Result<Status, ApiError> {
        status(&self.read())
    }

    pub(crate) fn read(&self) -> MutexGuard<'_, Fixture> {
        crate::api::lock(&self.world)
    }

    /// `POST /api/networks/:net/joins/:node/approve`: the device joins under `name`, or
    /// the name it asked for.
    pub(crate) fn approve(&self, live: &Live, net: &str, node: &str, name: Option<&str>) -> Answer {
        let mut world = self.read();
        let (network, node) = (known(&world, net)?, node_id(node)?);
        admin(&world, net)?;
        let Some(index) = world
            .pending
            .joins
            .iter()
            .position(|join| join.network == net && join.node == node.to_string())
        else {
            return Err(settled_join(&world, net, node));
        };
        let join = world.pending.joins.remove(index);
        let asked = name.filter(|name| !name.trim().is_empty());
        let name = free_name(&world, net, &normalize(asked.unwrap_or(&join.claimed.name)));
        let address = add_device(&mut world, net, node, &name, None)?;
        live.push(EntryKind::DeviceJoined {
            network,
            node,
            name,
            address,
            invite: None,
        });
        Ok(accepted())
    }

    /// `POST /api/networks/:net/joins/:node/deny`.
    pub(crate) fn deny(&self, net: &str, node: &str) -> Answer {
        let mut world = self.read();
        let node = node_id(node)?;
        known(&world, net)?;
        admin(&world, net)?;
        let before = world.pending.joins.len();
        world
            .pending
            .joins
            .retain(|join| !(join.network == net && join.node == node.to_string()));
        if world.pending.joins.len() == before {
            return Err(settled_join(&world, net, node));
        }
        Ok(accepted())
    }

    /// `POST /api/networks/:net/devices/:node/remove`; this device leaves instead.
    pub(crate) fn remove(&self, live: &Live, net: &str, node: &str, lost: bool) -> Answer {
        let node = node_id(node)?;
        if self.local(net)? == node {
            return self.leave(live, net);
        }
        let mut world = self.read();
        let network = known(&world, net)?;
        admin(&world, net)?;
        let mut peers = peers(&world, net)?;
        let Some(index) = peers.iter().position(|peer| peer.node == node) else {
            return Err(settled(None, now_ms(), None));
        };
        let gone = peers.remove(index);
        set_peers(&mut world, net, &peers)?;
        let device_name = format!("{}.{}", gone.name, suffix_of(&world, net));
        if let Some(records) = world.records.get_mut(net) {
            records.retain(|record| {
                let own = record.device && record.name == device_name;
                let points = !record.device
                    && (record.value == gone.address
                        || record.value.trim_end_matches('.') == device_name);
                !(own || (lost && points))
            });
        }
        if let Some(exits) = world.exits.get_mut(net) {
            exits.retain(|exit| exit.node != node.to_string());
        }
        let mut status = status(&world)?;
        if let Some(summary) = status.networks.iter_mut().find(|n| n.id == network) {
            summary.peers_total = summary.peers_total.saturating_sub(1);
            if gone.online {
                summary.peers_online = summary.peers_online.saturating_sub(1);
            }
        }
        let using = status.exit_node.as_deref() == Some(&exit_label(&world, net, &gone.name));
        if using {
            status.exit_node = None;
        }
        set_status(&mut world, &status)?;
        live.push(EntryKind::DeviceRemoved {
            network,
            node,
            name: gone.name,
        });
        if using {
            live.push(EntryKind::ExitChanged { exit: None });
        }
        Ok(accepted())
    }

    /// `POST /api/networks/:net/leave`: the network and everything about it goes.
    pub(crate) fn leave(&self, live: &Live, net: &str) -> Answer {
        let mut world = self.read();
        let network = known(&world, net)?;
        let peers = peers(&world, net)?;
        let mut status = status(&world)?;
        let name = status
            .networks
            .iter()
            .find(|summary| summary.id == network)
            .map(|summary| summary.name.clone())
            .unwrap_or_default();
        status.networks.retain(|summary| summary.id != network);
        let using = status
            .exit_node
            .as_deref()
            .is_some_and(|exit| exit.split_once('/').is_some_and(|(n, _)| n == name));
        if using {
            status.exit_node = None;
        }
        set_status(&mut world, &status)?;
        world.status.memberships.retain(|m| m.network != net);
        world.peers.remove(net);
        world.records.remove(net);
        world.exits.remove(net);
        world.invites.remove(net);
        world.relays.remove(net);
        world.policies.remove(net);
        world.links.remove(net);
        world.pending.joins.retain(|join| join.network != net);
        world.pending.blocked.retain(|group| group.network != net);
        drop(world);
        for peer in peers {
            live.push(EntryKind::DeviceRemoved {
                network,
                node: peer.node,
                name: peer.name,
            });
        }
        if using {
            live.push(EntryKind::ExitChanged { exit: None });
        }
        Ok(accepted())
    }

    /// `POST /api/networks/:net/devices/:node/rename`.
    pub(crate) fn rename(&self, live: &Live, net: &str, node: &str, name: &str) -> Answer {
        let node = node_id(node)?;
        let local = self.local(net)?;
        let mut world = self.read();
        let network = known(&world, net)?;
        admin(&world, net)?;
        let label = normalize(name);
        if let Some(problem) = problem(&label) {
            return Err(ApiError::bad(format!("not a usable name: {problem:?}")));
        }
        let suffix = suffix_of(&world, net);
        let taken = peers(&world, net)?
            .iter()
            .filter(|peer| peer.node != node)
            .map(|peer| peer.name.clone())
            .chain((node != local).then(|| local_name(&world.status.memberships, net)))
            .any(|taken| taken == label);
        if taken {
            return Err(ApiError::bad(format!("{label} is taken in this network")));
        }
        let old = if node == local {
            let membership = world
                .status
                .memberships
                .iter_mut()
                .find(|m| m.network == net)
                .ok_or_else(|| ApiError::not_found("this device is not in that network"))?;
            core::mem::replace(&mut membership.name, label.clone())
        } else {
            let mut peers = peers(&world, net)?;
            let peer = peers
                .iter_mut()
                .find(|peer| peer.node == node)
                .ok_or_else(|| ApiError::not_found("no such device in this network"))?;
            name.trim().clone_into(&mut peer.display_name);
            let old = core::mem::replace(&mut peer.name, label.clone());
            set_peers(&mut world, net, &peers)?;
            old
        };
        if old == label {
            return Ok(accepted());
        }
        for record in world.records.get_mut(net).into_iter().flatten() {
            if record.device && record.name == format!("{old}.{suffix}") {
                record.name = format!("{label}.{suffix}");
            }
        }
        for exit in world.exits.get_mut(net).into_iter().flatten() {
            if exit.node == node.to_string() {
                exit.name.clone_from(&label);
            }
        }
        let mut status = status(&world)?;
        if status.exit_node.as_deref() == Some(&exit_label(&world, net, &old)) {
            status.exit_node = Some(exit_label(&world, net, &label));
            set_status(&mut world, &status)?;
        }
        live.push(EntryKind::DeviceRenamed {
            network,
            node,
            old_name: old,
            name: label,
        });
        Ok(accepted())
    }

    /// `POST /api/networks/:net/records`.
    pub(crate) fn set_record(&self, net: &str, input: &RecordIn) -> Answer {
        let mut guard = self.read();
        let world = &mut *guard;
        known(world, net)?;
        admin(world, net)?;
        check_value(input.kind, &input.value)?;
        let suffix = suffix_of(world, net);
        let set_by = local_name(&world.status.memberships, net);
        let full = format!("{}.{suffix}", short(&input.name, &suffix));
        let records = world.records.entry(net.to_owned()).or_default();
        let current: Vec<String> = records
            .iter()
            .filter(|record| !record.device && record.name == full)
            .map(|record| record.value.clone())
            .collect();
        if let Some(based_on) = &input.based_on
            && !current.contains(based_on)
        {
            return Err(settled(None, now_ms(), current.into_iter().next()));
        }
        records.retain(|record| record.device || record.name != full);
        records.push(RecordView {
            name: full,
            kind: input.kind,
            value: input.value.clone(),
            device: false,
            set_by: Some(set_by),
        });
        Ok(accepted())
    }

    /// `POST /api/networks/:net/records/:name/delete`.
    pub(crate) fn delete_record(&self, net: &str, name: &str) -> Answer {
        let mut world = self.read();
        known(&world, net)?;
        admin(&world, net)?;
        let suffix = suffix_of(&world, net);
        let full = format!("{}.{suffix}", short(name, &suffix));
        if let Some(records) = world.records.get_mut(net) {
            records.retain(|record| record.device || record.name != full);
        }
        Ok(accepted())
    }

    /// `POST /api/exit`.
    pub(crate) fn use_exit(&self, live: &Live, input: &ExitIn) -> Answer {
        let mut world = self.read();
        let net = match (&input.network, &input.node) {
            (Some(net), _) => net.clone(),
            (None, Some(_)) => return Err(ApiError::bad("an exit needs its network")),
            (None, None) => world
                .status
                .memberships
                .first()
                .map(|m| m.network.clone())
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        ApiErrorCode::NotFound,
                        "this device is in no network",
                    )
                })?,
        };
        known(&world, &net)?;
        let me = local_name(&world.status.memberships, &net);
        let exit = match input.node.as_deref() {
            None => None,
            Some(node) => {
                let node = node_id(node)?.to_string();
                let name = world
                    .exits
                    .get(&net)
                    .into_iter()
                    .flatten()
                    .find(|exit| exit.node == node && exit.allowed)
                    .map(|exit| exit.name.clone())
                    .ok_or_else(|| ApiError::bad(format!("{node} is not an exit node")))?;
                Some(exit_label(&world, &net, &name))
            }
        };
        let status_value = world.status.status.clone();
        for (id, exits) in &mut world.exits {
            for one in exits {
                one.in_use_by.retain(|user| *user != me);
                if exit.as_deref() == Some(&exit_label_of(&one.name, id, &status_value)) {
                    one.in_use_by.push(me.clone());
                }
            }
        }
        let mut status = status(&world)?;
        status.exit_node.clone_from(&exit);
        set_status(&mut world, &status)?;
        live.push(EntryKind::ExitChanged { exit });
        Ok(accepted())
    }

    /// `POST /api/networks/:net/exits/:node`.
    pub(crate) fn grant(&self, net: &str, node: &str, allowed: bool) -> Answer {
        let mut world = self.read();
        known(&world, net)?;
        admin(&world, net)?;
        let node = node_id(node)?;
        let name = peers(&world, net)?
            .into_iter()
            .find(|peer| peer.node == node)
            .map(|peer| peer.name)
            .ok_or_else(|| ApiError::not_found("no such device in this network"))?;
        let exits = world.exits.entry(net.to_owned()).or_default();
        exits.retain(|exit| exit.node != node.to_string());
        if allowed {
            exits.push(ExitView {
                network: net.to_owned(),
                node: node.to_string(),
                name,
                allowed: true,
                access: Vec::new(),
                in_use_by: Vec::new(),
            });
        }
        Ok(accepted())
    }

    /// `POST /api/networks/:net/invites`: a real link for a network no daemon serves.
    pub(crate) fn create_invite(&self, net: &str, input: &InviteIn) -> Answer {
        let mut world = self.read();
        let network = known(&world, net)?;
        admin(&world, net)?;
        let now = now_ms();
        let expires_at_ms = now.saturating_add(input.expires_secs.saturating_mul(1000));
        let token = InviteToken::generate(
            network,
            self_node(&world)?,
            Vec::new(),
            Timestamp::from_millis(expires_at_ms),
            network_name(&world, network),
        )
        .map_err(|error| ApiError::bad(error.to_string()))?;
        let link = token
            .to_link()
            .map_err(|error| ApiError::bad(error.to_string()))?
            .to_string();
        let id = hex(&token.invite_public_key().to_bytes());
        world
            .invites
            .entry(net.to_owned())
            .or_default()
            .push(OpenInvite {
                id: id.clone(),
                expires_at_ms,
                uses_left: input.uses,
                created_at_ms: now,
            });
        Ok(Json(json!(Invite {
            id,
            qr_svg: qr_svg(&link),
            link,
            expires_at_ms,
            uses: input.uses,
            used_by: Vec::new(),
        })))
    }

    /// `GET /api/networks/:net/invites`: open invites, newest first.
    pub(crate) fn invites(&self, net: &str) -> Answer {
        let world = self.read();
        known(&world, net)?;
        let now = now_ms();
        let mut open: Vec<OpenInvite> = world
            .invites
            .get(net)
            .into_iter()
            .flatten()
            .filter(|invite| invite.expires_at_ms > now)
            .cloned()
            .collect();
        open.sort_by_key(|invite| core::cmp::Reverse(invite.created_at_ms));
        Ok(Json(json!(open)))
    }

    /// `POST /api/networks/:net/invites/:id/revoke`.
    pub(crate) fn revoke_invite(&self, net: &str, id: &str) -> Answer {
        let mut world = self.read();
        known(&world, net)?;
        admin(&world, net)?;
        let invites = world.invites.entry(net.to_owned()).or_default();
        let before = invites.len();
        invites.retain(|invite| invite.id != id);
        if invites.len() == before {
            return Err(ApiError::not_found(format!("invite {id}")));
        }
        Ok(accepted())
    }

    /// `POST /api/blocked/:id/allow`: the preview, or the rule added and the group gone.
    pub(crate) fn allow(&self, id: &str, preview: bool) -> Answer {
        let mut world = self.read();
        let group = world
            .pending
            .blocked
            .iter()
            .find(|group| group.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("no such blocked connection"))?;
        if preview {
            let rule = crate::api::blocked::sentence(
                &group.from_name,
                &group.to_name,
                &group.protocol,
                group.port,
            );
            return Ok(Json(json!(AllowPreview {
                new_paths: vec![rule.clone()],
                rule_sentence: rule,
            })));
        }
        admin(&world, &group.network)?;
        let rule = allow_rule(&Blocked {
            from: node_id(&group.from)?,
            to: node_id(&group.to)?,
            protocol: group.protocol.clone(),
            port: group.port,
        })?;
        let mut rules = policy_of(&world, &group.network)?.rules().to_vec();
        if !rules.contains(&rule) {
            rules.push(rule);
        }
        let policy = kith_policy::Policy::new(rules).map_err(|e| ApiError::bad(e.to_string()))?;
        let toml = toml::to_string(&policy).map_err(|e| ApiError::bad(e.to_string()))?;
        world.policies.insert(group.network, toml);
        world.pending.blocked.retain(|one| one.id != id);
        Ok(accepted())
    }

    /// `GET /api/networks/:net/policy`.
    pub(crate) fn policy(&self, net: &str) -> Answer {
        let world = self.read();
        known(&world, net)?;
        let policy = policy_of(&world, net)?;
        let toml = toml::to_string(&policy).map_err(|e| ApiError::bad(e.to_string()))?;
        let names: BTreeMap<String, String> = peers(&world, net)?
            .into_iter()
            .map(|peer| (peer.node.to_string(), peer.name))
            .chain(
                world
                    .status
                    .status
                    .get("node")
                    .and_then(Value::as_str)
                    .map(|node| (node.to_owned(), local_name(&world.status.memberships, net))),
            )
            .collect();
        Ok(Json(json!(describe(&policy, toml, &names))))
    }

    /// `POST /api/networks/:net/policy`: 400 with the parser's line and column.
    pub(crate) fn set_policy(&self, net: &str, input: &PolicyIn) -> Answer {
        let mut world = self.read();
        known(&world, net)?;
        admin(&world, net)?;
        toml::from_str::<kith_policy::Policy>(&input.toml)
            .map_err(|error| ApiError::bad(format!("policy: {error}")))?;
        world.policies.insert(net.to_owned(), input.toml.clone());
        Ok(accepted())
    }

    /// `GET /api/networks/:net/relays`.
    pub(crate) fn relays(&self, net: &str) -> Answer {
        let world = self.read();
        known(&world, net)?;
        Ok(Json(json!(
            world.relays.get(net).cloned().unwrap_or_default()
        )))
    }

    /// `POST /api/networks/:net/relays`: replaces a relay of the same name.
    pub(crate) fn add_relay(&self, net: &str, input: &RelayIn) -> Answer {
        let mut world = self.read();
        let network = known(&world, net)?;
        admin(&world, net)?;
        let name = input.name.trim().to_ascii_lowercase();
        if name.is_empty() || input.address.trim().is_empty() {
            return Err(ApiError::bad("a relay needs a name and an address"));
        }
        let node = match input.node.as_deref() {
            Some(node) => node_id(node)?,
            None => self_node(&world)?,
        };
        let relays = world.relays.entry(net.to_owned()).or_default();
        relays.retain(|relay| relay.name != name);
        relays.push(RelayView {
            name,
            address: input.address.trim().to_owned(),
            node: node.to_string(),
        });
        let count = u32::try_from(relays.len()).unwrap_or(u32::MAX);
        set_relay_count(&mut world, network, count)?;
        Ok(accepted())
    }

    /// `POST /api/networks/:net/relays/:name/remove`.
    pub(crate) fn remove_relay(&self, net: &str, name: &str) -> Answer {
        let mut world = self.read();
        let network = known(&world, net)?;
        admin(&world, net)?;
        let relays = world.relays.entry(net.to_owned()).or_default();
        let before = relays.len();
        relays.retain(|relay| relay.name != name);
        if relays.len() == before {
            return Err(ApiError::not_found(format!("relay {name}")));
        }
        let count = u32::try_from(relays.len()).unwrap_or(u32::MAX);
        set_relay_count(&mut world, network, count)?;
        Ok(accepted())
    }

    /// `GET /api/server`.
    pub(crate) fn server(&self) -> Answer {
        let status = status(&self.read())?;
        Ok(Json(json!(ServerView {
            exit: status.exit,
            relay: status.relay,
        })))
    }

    /// `POST /api/server`.
    pub(crate) fn set_server(&self, input: ServerView) -> Answer {
        let mut world = self.read();
        let mut status = status(&world)?;
        status.exit = input.exit;
        status.relay = input.relay;
        set_status(&mut world, &status)?;
        Ok(accepted())
    }

    /// `POST /api/wireguard/import`: checks the config as the daemon does and lists the
    /// network in use, replacing one of the same name. One that carries all internet
    /// traffic replaces the exit in use.
    pub(crate) fn import(&self, live: &Live, input: &ImportIn) -> Answer {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(ApiError::bad("import name is empty"));
        }
        let config = kith_config::WgQuickConfig::parse(&input.conf_text)
            .map_err(|error| ApiError::bad(format!("wg-quick: {error}")))?;
        let mut world = self.read();
        let mut status = status(&world)?;
        let network = status
            .imported
            .iter()
            .find(|import| import.name == name)
            .map_or_else(
                || random_id().map(|id| NetworkId::from_bytes(id.to_bytes())),
                |import| Ok(import.network),
            )?;
        let mut routes: Vec<_> = config
            .peers
            .iter()
            .flat_map(|peer| peer.allowed_ips.iter().copied())
            .collect();
        routes.sort_unstable();
        routes.dedup();
        let probes = [[1, 1, 1, 1], [203, 0, 113, 1]]
            .map(|octets| core::net::IpAddr::V4(core::net::Ipv4Addr::from(octets)));
        let everything = probes
            .iter()
            .all(|probe| routes.iter().any(|route| route.contains(*probe)));
        status.imported.retain(|import| import.name != name);
        status.imported.push(ImportedInfo {
            network,
            name: name.to_owned(),
            endpoint: config
                .peers
                .iter()
                .find_map(|peer| peer.endpoint.as_ref())
                .map(|endpoint| match endpoint {
                    kith_config::Endpoint::Addr(addr) => addr.to_string(),
                    kith_config::Endpoint::Host { host, port } => format!("{host}:{port}"),
                }),
            routes: routes.iter().map(ToString::to_string).collect(),
            everything,
            active: true,
            last_seen_secs: None,
            rx_bytes: 0,
            tx_bytes: 0,
        });
        set_status(&mut world, &status)?;
        drop(world);
        if everything {
            self.activate(live, network)?;
        }
        Ok(Json(json!(Imported {
            network: network.to_string(),
        })))
    }

    /// `POST /api/wireguard/:net/use`.
    pub(crate) fn use_import(&self, live: &Live, net: &str, active: bool) -> Answer {
        let network = network_id(net)?;
        {
            let mut world = self.read();
            let mut status = status(&world)?;
            let import = status
                .imported
                .iter_mut()
                .find(|import| import.network == network)
                .ok_or_else(|| ApiError::not_found("no such imported network"))?;
            import.active = active;
            set_status(&mut world, &status)?;
        }
        if active {
            self.activate(live, network)?;
        }
        Ok(accepted())
    }

    /// An import that carries all internet traffic was turned on: every other such
    /// import, and the exit in use, stop.
    fn activate(&self, live: &Live, network: NetworkId) -> Result<(), ApiError> {
        let mut world = self.read();
        let mut status = status(&world)?;
        if !status
            .imported
            .iter()
            .any(|import| import.network == network && import.everything)
        {
            return Ok(());
        }
        for import in &mut status.imported {
            if import.everything && import.network != network {
                import.active = false;
            }
        }
        let had_exit = status.exit_node.take().is_some();
        set_status(&mut world, &status)?;
        for exits in world.exits.values_mut() {
            for exit in exits {
                exit.in_use_by.clear();
            }
        }
        drop(world);
        if had_exit {
            live.push(EntryKind::ExitChanged { exit: None });
        }
        Ok(())
    }

    /// `POST /api/wireguard/:net/remove`.
    pub(crate) fn remove_import(&self, net: &str) -> Answer {
        let network = network_id(net)?;
        let mut world = self.read();
        let mut status = status(&world)?;
        let before = status.imported.len();
        status.imported.retain(|import| import.network != network);
        if status.imported.len() == before {
            return Err(ApiError::not_found("no such imported network"));
        }
        set_status(&mut world, &status)?;
        Ok(accepted())
    }

    /// `POST /api/networks`: a new network with this device as its only admin.
    pub(crate) fn create_network(&self, input: &CreateNetworkIn) -> Answer {
        let name = input.name.trim();
        if name.is_empty() || input.device_name.trim().is_empty() {
            return Err(ApiError::bad("a network needs a name and a device name"));
        }
        let mut world = self.read();
        let network = NetworkId::from_bytes(random_id()?.to_bytes());
        add_network(
            &mut world,
            network,
            name,
            &normalize(&input.device_name),
            Role::Admin,
        )?;
        Ok(accepted())
    }

    /// `POST /api/join`: joins the network an invite link names, as a member.
    pub(crate) fn join(&self, input: &JoinIn) -> Answer {
        let token = InviteToken::parse(input.invite_link.trim())
            .map_err(|error| ApiError::bad(format!("not an invite link: {error}")))?;
        let mut world = self.read();
        if status(&world)?
            .networks
            .iter()
            .any(|n| n.id == token.network())
        {
            return Err(ApiError::bad(format!("already joined {}", token.network())));
        }
        add_network(
            &mut world,
            token.network(),
            token.network_name(),
            &normalize(&input.device_name),
            Role::Member,
        )?;
        Ok(accepted())
    }

    /// `POST /api/join/request`: waits for an admin who is not there; the request stays
    /// pending.
    pub(crate) fn request_join(&self, code: &str) -> Answer {
        let code = RequestCode::parse(code.trim())
            .map_err(|error| ApiError::bad(format!("not a request code: {error}")))?;
        let world = self.read();
        Ok(Json(json!(JoinRequestState {
            network: Some(code.network().to_string()),
            network_name: code.network_name().to_owned(),
            check_code: check_code(&self_node(&world)?),
            resolution: None,
        })))
    }

    /// `POST /api/networks/:net/phones`: a phone joins through this device, and its made-up
    /// config is held for the page.
    /// The fixture's phone gateways: this device, an exit when it offers one.
    pub(crate) fn gateways(&self, net: &str) -> Result<Vec<kith_ipc::GatewayInfo>, ApiError> {
        let world = self.read();
        known(&world, net)?;
        let status = status(&world)?;
        let Some(node) = status.node else {
            return Ok(Vec::new());
        };
        Ok(vec![kith_ipc::GatewayInfo {
            node,
            name: local_name(&world.status.memberships, net),
            endpoint: "203.0.113.1:51820".to_owned(),
            exit: status.exit,
            default: true,
        }])
    }

    pub(crate) fn add_phone(
        &self,
        live: &Live,
        net: &str,
        input: &PhoneIn,
    ) -> Result<(PhoneCreated, HeldPhone), ApiError> {
        let mut world = self.read();
        let network = known(&world, net)?;
        admin(&world, net)?;
        let node = random_id()?;
        let name = free_name(&world, net, &normalize(&input.name));
        let gateway = local_name(&world.status.memberships, net);
        let address = add_device(&mut world, net, node, &name, Some(gateway.clone()))?;
        let config = format!(
            "# simulated by kith-ui --fixture; no daemon made this\n[Interface]\nAddress = {address}/32\n"
        );
        live.push(EntryKind::DeviceJoined {
            network,
            node,
            name: name.clone(),
            address,
            invite: None,
        });
        Ok((
            PhoneCreated {
                name,
                node: node.to_string(),
                gateway,
                everything_available: false,
            },
            HeldPhone {
                mesh: config,
                everything: None,
                made_ms: now_ms(),
                made_by: None,
            },
        ))
    }

    /// This device's id.
    fn local(&self, net: &str) -> Result<NodeId, ApiError> {
        let world = self.read();
        known(&world, net)?;
        self_node(&world)
    }
}

/// The network `net` names, if the recording holds it: 400 or 404 as a daemon answers.
fn known(world: &Fixture, net: &str) -> Result<NetworkId, ApiError> {
    crate::api::status::recorded_network(world, net)?;
    network_id(net)
}

/// 403 unless this device is an admin of `net`.
fn admin(world: &Fixture, net: &str) -> Result<(), ApiError> {
    let role = world
        .status
        .memberships
        .iter()
        .find(|m| m.network == net)
        .map(|m| m.role);
    if role == Some(Role::Admin) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::PermissionDenied,
            "only an admin of this network can do this",
        ))
    }
}

/// The 409 for a join request that is no longer pending.
fn settled_join(world: &Fixture, net: &str, node: NodeId) -> ApiError {
    let member = peers(world, net).is_ok_and(|peers| peers.iter().any(|p| p.node == node));
    let resolution = if member {
        Resolution::Approved
    } else {
        Resolution::Denied
    };
    settled(Some(resolution), now_ms(), None)
}

fn status(world: &Fixture) -> Result<Status, ApiError> {
    Ok(json::unshape(&world.status.status)?)
}

fn set_status(world: &mut Fixture, status: &Status) -> Result<(), ApiError> {
    world.status.status = json::to_value(status)?;
    Ok(())
}

fn peers(world: &Fixture, net: &str) -> Result<Vec<PeerInfo>, ApiError> {
    world
        .peers
        .get(net)
        .map_or_else(|| Ok(Vec::new()), |peers| Ok(json::unshape(peers)?))
}

fn set_peers(world: &mut Fixture, net: &str, peers: &[PeerInfo]) -> Result<(), ApiError> {
    world.peers.insert(net.to_owned(), json::to_value(&peers)?);
    Ok(())
}

fn self_node(world: &Fixture) -> Result<NodeId, ApiError> {
    world
        .status
        .status
        .get("node")
        .and_then(Value::as_str)
        .ok_or_else(ApiError::unexpected)
        .and_then(node_id)
}

fn local_name(memberships: &[Membership], net: &str) -> String {
    memberships
        .iter()
        .find(|m| m.network == net)
        .map(|m| m.name.clone())
        .unwrap_or_default()
}

fn network_name(world: &Fixture, network: NetworkId) -> String {
    status(world)
        .ok()
        .and_then(|status| {
            status
                .networks
                .into_iter()
                .find(|n| n.id == network)
                .map(|n| n.name)
        })
        .unwrap_or_default()
}

fn suffix_of(world: &Fixture, net: &str) -> String {
    suffix(&network_id(net).map_or_else(|_| String::new(), |id| network_name(world, id)))
}

/// `network/device`, as the status names the exit in use.
fn exit_label(world: &Fixture, net: &str, device: &str) -> String {
    exit_label_of(device, net, &world.status.status)
}

fn exit_label_of(device: &str, net: &str, status: &Value) -> String {
    let network = status
        .get("networks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|n| n.get("id").and_then(Value::as_str) == Some(net))
        .and_then(|n| n.get("name").and_then(Value::as_str))
        .unwrap_or_default();
    format!("{network}/{device}")
}

/// `name`, or the first free `name-2`, `name-3`, … in the network.
fn free_name(world: &Fixture, net: &str, name: &str) -> String {
    let taken: Vec<String> = peers(world, net)
        .unwrap_or_default()
        .into_iter()
        .map(|peer| peer.name)
        .chain(core::iter::once(local_name(&world.status.memberships, net)))
        .collect();
    (1..=999_u32)
        .map(|number| {
            if number == 1 {
                name.to_owned()
            } else {
                format!("{name}-{number}")
            }
        })
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| name.to_owned())
}

/// Adds a device that is not online yet, its name record and its count; returns its
/// address, the first free one in `100.64.0.0/24`.
fn add_device(
    world: &mut Fixture,
    net: &str,
    node: NodeId,
    name: &str,
    gateway: Option<String>,
) -> Result<String, ApiError> {
    let mut peers = peers(world, net)?;
    let used: Vec<String> = peers
        .iter()
        .map(|peer| peer.address.clone())
        .chain(world.status.memberships.iter().map(|m| m.address.clone()))
        .collect();
    let address = (1..=254_u8)
        .map(|host| format!("100.64.0.{host}"))
        .find(|address| !used.contains(address))
        .ok_or_else(|| ApiError::bad("the simulated network is full"))?;
    peers.push(PeerInfo {
        settings: None,
        via_this_exit: false,
        node,
        name: name.to_owned(),
        address: address.clone(),
        path: None,
        path_reason: None,
        last_seen_secs: None,
        endpoint: None,
        rx_bytes: 0,
        tx_bytes: 0,
        online: false,
        display_name: name.to_owned(),
        gateway,
        path_mtu: None,
        invite: None,
    });
    set_peers(world, net, &peers)?;
    let suffix = suffix_of(world, net);
    world
        .records
        .entry(net.to_owned())
        .or_default()
        .push(RecordView {
            name: format!("{name}.{suffix}"),
            kind: RecordType::A,
            value: address.clone(),
            device: true,
            set_by: None,
        });
    let network = network_id(net)?;
    let mut status = status(world)?;
    if let Some(summary) = status.networks.iter_mut().find(|n| n.id == network) {
        summary.peers_total = summary.peers_total.saturating_add(1);
    }
    set_status(world, &status)?;
    Ok(address)
}

/// A network this device has just created or joined: no other devices yet.
fn add_network(
    world: &mut Fixture,
    network: NetworkId,
    name: &str,
    device: &str,
    role: Role,
) -> Result<(), ApiError> {
    let me = self_node(world)?;
    let mut status = status(world)?;
    status.networks.push(kith_ipc::NetworkSummary {
        my_settings: None,
        id: network,
        name: name.to_owned(),
        peers_online: 0,
        peers_total: 0,
        relays: 0,
        problems: Vec::new(),
        admin: role == Role::Admin,
    });
    set_status(world, &status)?;
    let net = network.to_string();
    let address = "100.64.0.1".to_owned();
    world.status.memberships.push(Membership {
        network: net.clone(),
        name: device.to_owned(),
        address: address.clone(),
        role,
        key_fpr: key_fpr(&me),
        made_admin_by: None,
        made_admin_at_ms: None,
        admins: if role == Role::Admin {
            vec![device.to_owned()]
        } else {
            Vec::new()
        },
        share_links: true,
    });
    world.peers.insert(net.clone(), json!([]));
    world.records.insert(
        net,
        vec![RecordView {
            name: format!("{device}.{}", suffix(name)),
            kind: RecordType::A,
            value: address,
            device: true,
            set_by: None,
        }],
    );
    Ok(())
}

fn set_relay_count(world: &mut Fixture, network: NetworkId, count: u32) -> Result<(), ApiError> {
    let mut status = status(world)?;
    if let Some(summary) = status.networks.iter_mut().find(|n| n.id == network) {
        summary.relays = count;
    }
    set_status(world, &status)
}

/// `name` without the network's suffix, lowercase.
fn short(name: &str, suffix: &str) -> String {
    let name = name.trim().trim_end_matches('.').to_ascii_lowercase();
    name.strip_suffix(&suffix.to_ascii_lowercase())
        .and_then(|rest| rest.strip_suffix('.'))
        .map_or_else(|| name.clone(), str::to_owned)
}

fn check_value(kind: RecordType, value: &str) -> Result<(), ApiError> {
    let ok = match kind {
        RecordType::A => value.parse::<core::net::Ipv4Addr>().is_ok(),
        RecordType::Aaaa => value.parse::<core::net::Ipv6Addr>().is_ok(),
        RecordType::Cname | RecordType::Txt => !value.trim().is_empty(),
    };
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad(format!(
            "{value} is not a valid {kind:?} value"
        )))
    }
}

fn policy_of(world: &Fixture, net: &str) -> Result<kith_policy::Policy, ApiError> {
    world.policies.get(net).map_or_else(
        || Ok(kith_policy::Policy::default_allow_all()),
        |text| toml::from_str(text).map_err(|error| ApiError::bad(format!("policy: {error}"))),
    )
}

/// The rule that allows one blocked connection, as the daemon writes it.
fn allow_rule(blocked: &Blocked) -> Result<Rule, ApiError> {
    let (protocol, ports) = match (blocked.protocol.as_str(), blocked.port) {
        ("tcp", Some(port)) => (Protocol::Tcp, PortSet::single(port)),
        ("udp", Some(port)) => (Protocol::Udp, PortSet::single(port)),
        ("icmp", _) => (Protocol::Icmp, PortSet::All),
        _ => (Protocol::Any, PortSet::All),
    };
    let target = Target::new(Selector::Device(blocked.to), protocol, ports)
        .map_err(|error| ApiError::bad(error.to_string()))?;
    Rule::new(vec![Selector::Device(blocked.from)], vec![target])
        .map_err(|error| ApiError::bad(error.to_string()))
}

/// The policy as `GET /api/networks/:net/policy` answers, devices named from `names`.
fn describe(
    policy: &kith_policy::Policy,
    toml: String,
    names: &BTreeMap<String, String>,
) -> PolicyInfo {
    let selector = |selector: &Selector| match selector {
        Selector::Any => SelectorInfo {
            kind: "any".to_owned(),
            value: None,
            name: None,
        },
        Selector::Device(node) => SelectorInfo {
            kind: "device".to_owned(),
            value: Some(node.to_string()),
            name: names.get(&node.to_string()).cloned(),
        },
        Selector::Tag(tag) => SelectorInfo {
            kind: "tag".to_owned(),
            value: Some(tag.to_string()),
            name: None,
        },
    };
    let rules = policy
        .rules()
        .iter()
        .map(|rule| PolicyRuleInfo {
            sources: rule.sources().iter().map(selector).collect(),
            destinations: rule
                .destinations()
                .iter()
                .map(|target| TargetInfo {
                    selector: selector(target.selector()),
                    protocol: match target.protocol() {
                        Protocol::Any => "any",
                        Protocol::Tcp => "tcp",
                        Protocol::Udp => "udp",
                        Protocol::Icmp => "icmp",
                    }
                    .to_owned(),
                    ports: match target.ports() {
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
                    },
                })
                .collect(),
        })
        .collect();
    PolicyInfo { toml, rules }
}

/// A fresh random id, for devices and networks the simulator makes up.
fn random_id() -> Result<NodeId, ApiError> {
    let mut bytes = [0_u8; 32];
    kith_crypto::fill_random(&mut bytes).map_err(|_| ApiError::unexpected())?;
    Ok(NodeId::from_bytes(bytes))
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

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use std::path::Path;

    use super::*;

    type TestResult = Result<(), Box<dyn core::error::Error>>;

    const NET: &str = "5f4e2f4c726e5fa085b73b4ae6ea05e4f15c97d84bc36f7497b32cc69c2ddf90";
    const PIXEL: &str = "66bceba6b0b666c022818615fb66b21eb9500cab887c356408befaa2bcd991a1";

    fn office() -> Result<Sim, Box<dyn core::error::Error>> {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../kith-ui/web/fixtures/office.json");
        Ok(Sim::new(crate::load_fixture(&path)?))
    }

    fn names(sim: &Sim) -> Result<Vec<String>, ApiError> {
        Ok(peers(&sim.read(), NET)?
            .into_iter()
            .map(|p| p.name)
            .collect())
    }

    #[test]
    fn each_change_edits_the_recording_and_writes_its_entry() -> TestResult {
        let sim = office()?;
        let live = Live::new("b0");
        let pixel = node_id(PIXEL)?;

        drop(sim.approve(&live, NET, PIXEL, None)?);
        assert!(names(&sim)?.contains(&"pixel-8".to_owned()));
        assert_eq!(sim.read().pending.joins.len(), 0);
        assert!(
            sim.approve(&live, NET, PIXEL, None)
                .is_err_and(|e| e.code() == ApiErrorCode::Conflict)
        );

        drop(sim.rename(&live, NET, PIXEL, "Lounge TV")?);
        assert!(names(&sim)?.contains(&"lounge-tv".to_owned()));
        assert!(
            sim.read().records[NET]
                .iter()
                .any(|r| r.name == "lounge-tv.home.kith.internal")
        );

        drop(sim.remove(&live, NET, PIXEL, false)?);
        assert!(!names(&sim)?.contains(&"lounge-tv".to_owned()));

        let vps = sim.read().exits[NET][0].node.clone();
        drop(sim.use_exit(
            &live,
            &ExitIn {
                network: Some(NET.to_owned()),
                node: None,
            },
        )?);
        assert_eq!(status(&sim.read())?.exit_node, None);
        drop(sim.use_exit(
            &live,
            &ExitIn {
                network: Some(NET.to_owned()),
                node: Some(vps),
            },
        )?);
        assert_eq!(status(&sim.read())?.exit_node.as_deref(), Some("home/vps"));

        let network = network_id(NET)?;
        assert_eq!(
            live.kinds(),
            [
                EntryKind::DeviceJoined {
                    network,
                    node: pixel,
                    name: "pixel-8".into(),
                    address: "100.64.0.13".into(),
                    invite: None,
                },
                EntryKind::DeviceRenamed {
                    network,
                    node: pixel,
                    old_name: "pixel-8".into(),
                    name: "lounge-tv".into(),
                },
                EntryKind::DeviceRemoved {
                    network,
                    node: pixel,
                    name: "lounge-tv".into(),
                },
                EntryKind::ExitChanged { exit: None },
                EntryKind::ExitChanged {
                    exit: Some("home/vps".into()),
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn invites_policy_relays_and_server_mode_round_trip() -> TestResult {
        let sim = office()?;
        let Json(invite) = sim.create_invite(
            NET,
            &InviteIn {
                expires_secs: 600,
                uses: Some(3),
            },
        )?;
        let id = invite["id"].as_str().ok_or("no id")?.to_owned();
        let link = invite["link"].as_str().ok_or("no link")?;
        assert_eq!(
            hex(&InviteToken::parse(link)?.invite_public_key().to_bytes()),
            id
        );
        let Json(open) = sim.invites(NET)?;
        assert_eq!(open[0]["id"], id.as_str());
        assert_eq!(open.as_array().map(Vec::len), Some(2));
        drop(sim.revoke_invite(NET, &id)?);
        assert_eq!(sim.invites(NET)?.0.as_array().map(Vec::len), Some(1));

        let blocked = sim.read().pending.blocked[0].id.clone();
        let Json(before) = sim.policy(NET)?;
        drop(sim.allow(&blocked, false)?);
        assert_eq!(sim.read().pending.blocked.len(), 0);
        let Json(after) = sim.policy(NET)?;
        let count = |policy: &Value| policy["rules"].as_array().map_or(0, Vec::len);
        assert_eq!(count(&after), count(&before) + 1);
        assert_eq!(after["rules"][2]["sources"][0]["name"], "phone");
        assert_eq!(after["rules"][2]["destinations"][0]["ports"], json!(["22"]));

        let bad = sim.set_policy(
            NET,
            &PolicyIn {
                toml: "[[rules]\n".into(),
            },
        );
        assert!(bad.is_err_and(|e| e.code() == ApiErrorCode::InvalidArgument));

        drop(sim.add_relay(
            NET,
            &RelayIn {
                name: "Edge".into(),
                address: "edge.example:443".into(),
                node: None,
            },
        )?);
        drop(sim.remove_relay(NET, "vps")?);
        let Json(relays) = sim.relays(NET)?;
        assert_eq!(relays.as_array().map(Vec::len), Some(1));
        assert_eq!(relays[0]["name"], "edge");
        assert_eq!(status(&sim.read())?.networks[0].relays, 1);

        drop(sim.set_server(ServerView {
            exit: true,
            relay: false,
        })?);
        assert_eq!(sim.server()?.0, json!({ "exit": true, "relay": false }));
        Ok(())
    }
}
