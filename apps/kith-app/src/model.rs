//! What the tray shows, worked out from the page server's snapshot: the status lines, the
//! icon, the exit choices and what needs this person. Pure, so it is tested without a
//! window, a tray or a daemon.

use core::fmt::Write as _;
use std::collections::BTreeMap;

use kith_ui_core::wire::{ApiErrorCode, ExitIn, ExitState, ExitView, Membership, Pending, Role};
use serde::Deserialize;
use serde_json::Value;

use crate::client::Failure;
use crate::copy;
use crate::icon::IconState;

/// The parts of `GET /api/boot` the shell reads.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct BootIn {
    /// The daemon's run.
    pub(crate) boot: String,
    /// The last journal entry the snapshot includes.
    pub(crate) seq: u64,
    /// The server's clock, which journal and request times are on.
    pub(crate) server_time_ms: u64,
    /// The daemon's status.
    pub(crate) status: StatusIn,
    /// Devices per network id.
    pub(crate) peers: BTreeMap<String, Vec<PeerIn>>,
    /// Everything waiting for a person.
    pub(crate) pending: Pending,
}

/// The parts of the status the shell reads.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct StatusIn {
    /// The networks this device is in.
    pub(crate) networks: Vec<NetworkIn>,
    /// The exit in use, as `network/device` names.
    pub(crate) exit_node: Option<String>,
    /// Problems the daemon hit; each needs a person.
    #[serde(default)]
    pub(crate) warnings: Vec<Value>,
    /// This device in each network.
    pub(crate) memberships: Vec<Membership>,
    /// How the exit in use is doing.
    pub(crate) exit_state: Option<ExitState>,
}

/// One network in the status.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct NetworkIn {
    /// Network id.
    pub(crate) id: String,
    /// Its name.
    pub(crate) name: String,
    /// Other devices online.
    pub(crate) peers_online: u64,
    /// Other devices.
    pub(crate) peers_total: u64,
    /// Problems in this network; each needs a person.
    #[serde(default)]
    pub(crate) problems: Vec<Value>,
}

/// One device in a network.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct PeerIn {
    /// Device id.
    pub(crate) node: String,
    /// Its name.
    pub(crate) name: String,
    /// Whether it is online.
    pub(crate) online: bool,
}

/// What the page server said, last time the shell asked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Reach {
    /// It answered.
    Up(Box<Snapshot>),
    /// It could not reach the daemon.
    Down(Down),
}

/// A snapshot plus the exits of each network.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    /// The snapshot.
    pub(crate) boot: BootIn,
    /// Exits per network id; a network whose exits could not be read has none.
    pub(crate) exits: BTreeMap<String, Vec<ExitView>>,
}

/// Why the daemon is out of reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Down {
    /// It is not running.
    NotRunning,
    /// It runs, but this OS user may not talk to it.
    NotInGroup,
    /// It speaks another protocol version.
    VersionSkew,
}

impl Down {
    /// Reads a failed `GET /api/boot`.
    pub(crate) fn from_failure(failure: &Failure) -> Self {
        match failure.code {
            Some(ApiErrorCode::NotInGroup) => Self::NotInGroup,
            Some(ApiErrorCode::VersionSkew) => Self::VersionSkew,
            _ => Self::NotRunning,
        }
    }
}

/// One exit choice in the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExitChoice {
    /// What the item says.
    pub(crate) label: String,
    /// The exit's name; `None` for no exit.
    pub(crate) name: Option<String>,
    /// What choosing it sends to `POST /api/exit`.
    pub(crate) choice: ExitIn,
    /// Whether it is the exit in use.
    pub(crate) checked: bool,
}

/// What needs this person, as a tray item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Needs {
    /// What the item says.
    pub(crate) label: String,
    /// The page path it opens.
    pub(crate) target: String,
}

/// Everything the tray shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Menu {
    /// The icon.
    pub(crate) icon: IconState,
    /// One line per network, or one line saying why kith is off.
    pub(crate) status: Vec<String>,
    /// The page path the status lines open.
    pub(crate) home: String,
    /// The exit choices; empty when no exit is available.
    pub(crate) exits: Vec<ExitChoice>,
    /// What needs this person, when anything does.
    pub(crate) needs: Option<Needs>,
}

/// How the page addresses a network: by name, or by the start of its id when two networks
/// share the name.
pub(crate) fn slug(networks: &[NetworkIn], network: &NetworkIn) -> String {
    let shared = networks
        .iter()
        .filter(|other| other.name == network.name)
        .count();
    if shared > 1 {
        network.id.chars().take(SHORT_ID).collect()
    } else {
        network.name.clone()
    }
}

/// Characters of a network id that tell networks with the same name apart.
const SHORT_ID: usize = 8;

/// A page path in a network, such as `/n/home/exits`.
pub(crate) fn network_path(networks: &[NetworkIn], network: &NetworkIn, rest: &str) -> String {
    format!("/n/{}{rest}", encode(&slug(networks, network)))
}

/// Percent-encodes a path segment or query value.
pub(crate) fn encode(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _infallible = write!(out, "%{byte:02X}");
        }
        out
    })
}

fn is_admin(boot: &BootIn, network: &str) -> bool {
    boot.status
        .memberships
        .iter()
        .any(|member| member.network == network && member.role == Role::Admin)
}

/// One thing that needs this person, and where the page shows it.
struct Need<'a> {
    network: &'a NetworkIn,
    id: Option<&'a str>,
}

/// What needs this person: join requests in networks this device is admin of, blocked
/// connections, problems and warnings (spec §5 `NeedsYouCard`).
fn needs(boot: &BootIn) -> Vec<Need<'_>> {
    let networks = &boot.status.networks;
    let find = |id: &str| networks.iter().find(|network| network.id == id);
    let mut found = Vec::new();
    for join in &boot.pending.joins {
        if let Some(network) = find(&join.network).filter(|_| is_admin(boot, &join.network)) {
            found.push(Need {
                network,
                id: Some(&join.id),
            });
        }
    }
    for blocked in &boot.pending.blocked {
        if let Some(network) = find(&blocked.network) {
            found.push(Need {
                network,
                id: Some(&blocked.id),
            });
        }
    }
    for network in networks {
        found.extend(network.problems.iter().map(|_| Need { network, id: None }));
    }
    if let Some(network) = networks.first() {
        found.extend(
            boot.status
                .warnings
                .iter()
                .map(|_| Need { network, id: None }),
        );
    }
    found
}

fn exit_choices(snapshot: &Snapshot) -> Vec<ExitChoice> {
    let networks = &snapshot.boot.status.networks;
    let current = snapshot.boot.status.exit_node.as_deref();
    let several = networks.len() > 1;
    let mut choices: Vec<ExitChoice> = networks
        .iter()
        .flat_map(|network| {
            snapshot
                .exits
                .get(&network.id)
                .into_iter()
                .flatten()
                .filter(|exit| exit.allowed)
                .map(move |exit| ExitChoice {
                    label: copy::exit_choice(&exit.name, several.then_some(network.name.as_str())),
                    name: Some(exit.name.clone()),
                    choice: ExitIn {
                        network: Some(network.id.clone()),
                        node: Some(exit.node.clone()),
                    },
                    checked: current == Some(format!("{}/{}", network.name, exit.name).as_str()),
                })
        })
        .collect();
    if choices.is_empty() {
        return choices;
    }
    choices.insert(
        0,
        ExitChoice {
            label: copy::NO_EXIT.to_owned(),
            name: None,
            choice: ExitIn {
                network: None,
                node: None,
            },
            checked: current.is_none(),
        },
    );
    choices
}

/// The tray for what the page server said.
pub(crate) fn menu(reach: &Reach) -> Menu {
    let off = |line: String| Menu {
        icon: IconState::Off,
        status: vec![line],
        home: "/".to_owned(),
        exits: Vec::new(),
        needs: None,
    };
    let snapshot = match reach {
        Reach::Down(Down::NotRunning) => return off(copy::DAEMON_DOWN.to_owned()),
        Reach::Down(Down::NotInGroup) => return off(copy::NOT_IN_GROUP.to_owned()),
        Reach::Down(Down::VersionSkew) => return off(copy::VERSION_SKEW.to_owned()),
        Reach::Up(snapshot) => snapshot,
    };
    let boot = &snapshot.boot;
    let networks = &boot.status.networks;
    let Some(first) = networks.first() else {
        return off(copy::NO_NETWORK.to_owned());
    };
    let status = networks
        .iter()
        .map(|network| {
            copy::connected(
                &network.name,
                network.peers_online.saturating_add(1),
                network.peers_total.saturating_add(1),
            )
        })
        .collect();
    let found = needs(boot);
    let needs = found.first().map(|need| {
        let target = match (found.len(), need.id) {
            (1, Some(id)) => network_path(networks, need.network, &format!("?need={}", encode(id))),
            _ => network_path(networks, need.network, ""),
        };
        Needs {
            label: copy::needs_you(found.len()),
            target,
        }
    });
    Menu {
        icon: if needs.is_some() {
            IconState::NeedsYou
        } else {
            IconState::Fine
        },
        status,
        home: network_path(networks, first, ""),
        exits: exit_choices(snapshot),
        needs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::office_snapshot;

    #[test]
    fn the_office_network_reads_as_the_spec_shows_it() {
        let snapshot = office_snapshot();
        let menu = menu(&Reach::Up(Box::new(snapshot.clone())));
        assert_eq!(menu.status, ["● Connected to home · 11 of 12 devices"]);
        assert_eq!(menu.home, "/n/home");
        assert_eq!(menu.icon, IconState::NeedsYou);
        let needs = menu.needs.expect("the office has a join request");
        assert_eq!(
            needs.label, "3 need you…",
            "a join, a blocked connection, a warning"
        );
        assert_eq!(needs.target, "/n/home");

        let labels: Vec<(&str, bool)> = menu
            .exits
            .iter()
            .map(|exit| (exit.label.as_str(), exit.checked))
            .collect();
        assert_eq!(
            labels.first(),
            Some(&("No exit · this device's own connection", false))
        );
        assert!(labels.contains(&("vps", true)));
        let allowed = snapshot
            .exits
            .values()
            .flatten()
            .filter(|exit| exit.allowed)
            .count();
        assert_eq!(menu.exits.len(), allowed + 1);
    }

    #[test]
    fn one_join_request_opens_its_card() {
        let mut snapshot = office_snapshot();
        snapshot.boot.pending.blocked.clear();
        snapshot.boot.status.warnings.clear();
        let id = snapshot.boot.pending.joins[0].id.clone();
        let menu = menu(&Reach::Up(Box::new(snapshot.clone())));
        let needs = menu.needs.expect("one request");
        assert_eq!(needs.label, "1 needs you…");
        assert_eq!(needs.target, format!("/n/home?need={}", encode(&id)));

        for member in &mut snapshot.boot.status.memberships {
            member.role = Role::Member;
        }
        let member = super::menu(&Reach::Up(Box::new(snapshot)));
        assert_eq!(
            member.needs, None,
            "members cannot approve, so a request does not need them"
        );
        assert_eq!(member.icon, IconState::Fine);
    }

    #[test]
    fn kith_off_reads_as_off() {
        let down = menu(&Reach::Down(Down::NotRunning));
        assert_eq!(down.icon, IconState::Off);
        assert_eq!(down.status, ["○ kith isn't running on this device"]);
        assert!(down.exits.is_empty() && down.needs.is_none());

        let mut snapshot = office_snapshot();
        snapshot.boot.status.networks.clear();
        let alone = menu(&Reach::Up(Box::new(snapshot)));
        assert_eq!(alone.icon, IconState::Off);
        assert_eq!(alone.status, [copy::NO_NETWORK]);
    }

    #[test]
    fn networks_sharing_a_name_are_told_apart_by_id() {
        let mut snapshot = office_snapshot();
        let mut twin = snapshot.boot.status.networks[0].clone();
        twin.id = "0123456789abcdef".to_owned();
        snapshot.boot.status.networks.push(twin);
        let networks = &snapshot.boot.status.networks;
        assert_eq!(
            network_path(networks, &networks[1], "/exits"),
            "/n/01234567/exits"
        );
        assert_eq!(encode("join:a b"), "join%3Aa%20b");
    }
}
