//! Native notifications (design spec §7.12, §15 M2): what notifies, the rules that keep it
//! quiet, and showing one with its only action, `Review`, which opens the page where the
//! thing is.
//!
//! Notifies: a join request (admins only), the exit in use going offline, a recently used
//! device going offline, and the daemon stopped for more than 10 s. Never: path changes,
//! names, logs, devices coming online. Never while the window is focused, never for
//! anything older than 30 s, and more than 3 in 30 s become one.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kith_ui_core::wire::{ExitState, Role, Tick};

use crate::copy;
use crate::model::{Snapshot, encode, network_path};
use crate::timing::{
    NOTIFY_COALESCE_COUNT, NOTIFY_COALESCE_WINDOW_MS, NOTIFY_REPLAY_CUTOFF_MS, RECENTLY_USED_MS,
};

/// The longest name a notification shows, in characters.
const MAX_NAME: usize = 64;
/// The most notifications waiting at once for the person to review or dismiss them.
/// Each waits on a thread of its own and one left in the notification centre waits
/// indefinitely, so past this they show without `Review`.
pub(crate) const MAX_WAITING: usize = 4;

/// `name` as a notification shows it. Device names come from the devices themselves, so
/// control and bidirectional formatting characters, which could forge a line or reorder
/// the text, are dropped, and a long name is cut.
pub(crate) fn shown_name(name: &str) -> String {
    let forged = |c: char| {
        c.is_control()
            || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    };
    let mut kept = name.chars().filter(|&c| !forged(c));
    let mut shown: String = kept.by_ref().take(MAX_NAME).collect();
    if kept.next().is_some() {
        shown.push('…');
    }
    shown
}

/// Counts the notifications waiting for the person, up to [`MAX_WAITING`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Waiting(Arc<AtomicUsize>);

/// A claimed place among the waiting notifications, given back when dropped.
#[derive(Debug)]
pub(crate) struct Place(Arc<AtomicUsize>);

impl Waiting {
    /// A place to wait in, or `None` when [`MAX_WAITING`] already wait.
    pub(crate) fn claim(&self) -> Option<Place> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_WAITING).then(|| count.saturating_add(1))
            })
            .ok()
            .map(|_| Place(Arc::clone(&self.0)))
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _was = self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    /// The kind and subject; one of each is shown per window.
    pub(crate) tag: String,
    /// The first line.
    pub(crate) title: String,
    /// The second line, when there is one.
    pub(crate) body: Option<String>,
    /// The page path `Review` opens.
    pub(crate) target: String,
}

/// When each device last moved traffic, from the ticks.
#[derive(Debug, Default)]
pub(crate) struct RecentUse {
    /// Per (network, device): the last counters and when they last changed.
    seen: BTreeMap<(String, String), ((u64, u64), u64)>,
}

impl RecentUse {
    /// Takes a tick's counters.
    pub(crate) fn tick(&mut self, tick: &Tick) {
        for traffic in &tick.traffic {
            let key = (traffic.network.clone(), traffic.node.clone());
            let counters = (traffic.rx, traffic.tx);
            match self.seen.get_mut(&key) {
                Some((last, _)) if *last == counters => {}
                Some(entry) => *entry = (counters, tick.at_ms),
                None => {
                    self.seen.insert(key, (counters, 0));
                }
            }
        }
    }

    /// Whether the device moved traffic within [`RECENTLY_USED_MS`] of `now_ms`.
    pub(crate) fn recent(&self, network: &str, node: &str, now_ms: u64) -> bool {
        self.seen
            .get(&(network.to_owned(), node.to_owned()))
            .is_some_and(|&(_, changed)| {
                changed > 0 && now_ms.saturating_sub(changed) <= RECENTLY_USED_MS
            })
    }
}

/// What changed between two snapshots that should notify.
pub(crate) fn notices(
    before: Option<&Snapshot>,
    after: &Snapshot,
    used: &RecentUse,
) -> Vec<Notice> {
    let boot = &after.boot;
    let now = boot.server_time_ms;
    let networks = &boot.status.networks;
    let network = |id: &str| networks.iter().find(|network| network.id == id);
    let admin = |id: &str| {
        boot.status
            .memberships
            .iter()
            .any(|member| member.network == id && member.role == Role::Admin)
    };
    let mut found = Vec::new();

    for join in &boot.pending.joins {
        let known = before.is_some_and(|before| {
            before
                .boot
                .pending
                .joins
                .iter()
                .any(|old| old.id == join.id)
        });
        let fresh = now.saturating_sub(join.requested_at_ms) <= NOTIFY_REPLAY_CUTOFF_MS;
        if let Some(net) =
            network(&join.network).filter(|_| !known && fresh && admin(&join.network))
        {
            found.push(Notice {
                tag: format!("join:{}", join.id),
                title: copy::notify_join(&shown_name(&join.claimed.name), &shown_name(&net.name)),
                body: Some(copy::NOTIFY_JOIN_BODY.to_owned()),
                target: network_path(networks, net, &format!("?need={}", encode(&join.id))),
            });
        }
    }

    if let Some(before) = before {
        let was_active = before.boot.status.exit_state == Some(ExitState::Active);
        let body = match boot.status.exit_state {
            Some(ExitState::OfflineBlocked) => Some(copy::EXIT_OFFLINE_BLOCKED),
            Some(ExitState::OfflineDirect) => Some(copy::EXIT_OFFLINE_DIRECT),
            Some(ExitState::Active) | None => None,
        };
        let exit = boot
            .status
            .exit_node
            .as_deref()
            .and_then(|exit| exit.split_once('/'));
        if let (true, Some(body), Some((net_name, exit))) = (was_active, body, exit) {
            let target = networks
                .iter()
                .find(|net| net.name == net_name)
                .map_or_else(
                    || "/".to_owned(),
                    |net| network_path(networks, net, "/exits"),
                );
            found.push(Notice {
                tag: format!("exit:{net_name}/{exit}"),
                title: copy::notify_exit_offline(&shown_name(exit)),
                body: Some(body.to_owned()),
                target,
            });
        }

        for (id, peers) in &boot.peers {
            let Some(net) = network(id) else { continue };
            for peer in peers.iter().filter(|peer| !peer.online) {
                let was_online =
                    before.boot.peers.get(id).is_some_and(|old| {
                        old.iter().any(|old| old.node == peer.node && old.online)
                    });
                if was_online && used.recent(id, &peer.node, now) {
                    found.push(Notice {
                        tag: format!("offline:{id}:{}", peer.node),
                        title: copy::notify_device_offline(&shown_name(&peer.name)),
                        body: None,
                        target: network_path(
                            networks,
                            net,
                            &format!("/devices/{}", encode(&peer.name)),
                        ),
                    });
                }
            }
        }
    }
    found
}

/// The daemon stopped for more than 10 s.
pub(crate) fn daemon_stopped() -> Notice {
    Notice {
        tag: "daemon".to_owned(),
        title: copy::NOTIFY_DAEMON_STOPPED.to_owned(),
        body: None,
        target: "/".to_owned(),
    }
}

/// Lets notifications through by the rules of §7.12.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    /// When each shown notification was shown, oldest first.
    shown: VecDeque<u64>,
    /// When each tag was last let through or held.
    tags: BTreeMap<String, u64>,
    /// Notifications held back since the window filled.
    held: usize,
}

impl Gate {
    fn prune(&mut self, now_ms: u64) {
        while self
            .shown
            .front()
            .is_some_and(|&at| now_ms.saturating_sub(at) >= NOTIFY_COALESCE_WINDOW_MS)
        {
            self.shown.pop_front();
        }
        self.tags
            .retain(|_, at| now_ms.saturating_sub(*at) < NOTIFY_COALESCE_WINDOW_MS);
    }

    /// Whether `notice` shows now. `focused` is whether the window has focus.
    pub(crate) fn admit(&mut self, notice: Notice, now_ms: u64, focused: bool) -> Option<Notice> {
        self.prune(now_ms);
        if focused || self.tags.contains_key(&notice.tag) {
            return None;
        }
        self.tags.insert(notice.tag.clone(), now_ms);
        if self.shown.len() >= NOTIFY_COALESCE_COUNT {
            self.held = self.held.saturating_add(1);
            return None;
        }
        self.shown.push_back(now_ms);
        Some(notice)
    }

    /// Once the window has passed, the held notifications as one, if any were held.
    pub(crate) fn flush(&mut self, now_ms: u64, focused: bool) -> Option<Notice> {
        self.prune(now_ms);
        if self.held == 0 || self.shown.len() >= NOTIFY_COALESCE_COUNT {
            return None;
        }
        let held = core::mem::take(&mut self.held);
        if focused {
            return None;
        }
        self.shown.push_back(now_ms);
        Some(Notice {
            tag: "coalesced".to_owned(),
            title: copy::notify_coalesced(held),
            body: None,
            target: "/".to_owned(),
        })
    }
}

/// Shows `notice` and blocks until the person reviews or dismisses it; true when they
/// chose `Review` or clicked it.
#[cfg(target_os = "macos")]
pub(crate) fn show_and_wait(identifier: &str, notice: &Notice) -> bool {
    use mac_notification_sys::{MainButton, Notification, NotificationResponse};
    let _already_set = mac_notification_sys::set_application(identifier);
    let response = Notification::new()
        .title(&notice.title)
        .message(notice.body.as_deref().unwrap_or_default())
        .main_button(MainButton::SingleAction(copy::REVIEW))
        .send();
    matches!(
        response,
        Ok(NotificationResponse::ActionButton(_) | NotificationResponse::Click)
    )
}

/// Shows `notice` and blocks until the person reviews or dismisses it; true when they
/// chose `Review` or clicked it.
#[cfg(target_os = "linux")]
pub(crate) fn show_and_wait(_identifier: &str, notice: &Notice) -> bool {
    /// The freedesktop action a click on the notification itself sends.
    const CLICK: &str = "default";
    /// The `Review` button's action.
    const REVIEW: &str = "review";
    let shown = notify_rust::Notification::new()
        .appname(copy::APP)
        .summary(&notice.title)
        .body(notice.body.as_deref().unwrap_or_default())
        .action(CLICK, copy::REVIEW)
        .action(REVIEW, copy::REVIEW)
        .show();
    let mut reviewed = false;
    if let Ok(handle) = shown {
        handle.wait_for_action(|action| reviewed = action == CLICK || action == REVIEW);
    }
    reviewed
}

/// Native notifications are not built for this platform yet.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn show_and_wait(_identifier: &str, _notice: &Notice) -> bool {
    false
}

/// Shows `notice` without `Review` and without waiting for the person.
#[cfg(target_os = "macos")]
pub(crate) fn show_only(identifier: &str, notice: &Notice) {
    use mac_notification_sys::Notification;
    let _already_set = mac_notification_sys::set_application(identifier);
    let _not_shown = Notification::new()
        .title(&notice.title)
        .message(notice.body.as_deref().unwrap_or_default())
        .asynchronous(true)
        .send();
}

/// Shows `notice` without `Review` and without waiting for the person.
#[cfg(target_os = "linux")]
pub(crate) fn show_only(_identifier: &str, notice: &Notice) {
    let _not_shown = notify_rust::Notification::new()
        .appname(copy::APP)
        .summary(&notice.title)
        .body(notice.body.as_deref().unwrap_or_default())
        .show();
}

/// Native notifications are not built for this platform yet.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn show_only(_identifier: &str, _notice: &Notice) {}

#[cfg(test)]
mod tests {
    use kith_ui_core::wire::TrafficTick;

    use super::*;
    use crate::tests::office_snapshot;

    fn notice(tag: &str) -> Notice {
        Notice {
            tag: tag.to_owned(),
            title: tag.to_owned(),
            body: None,
            target: "/".to_owned(),
        }
    }

    #[test]
    fn a_new_join_request_notifies_admins_once() {
        let before = office_snapshot();
        let mut after = before.clone();
        let mut join = after.boot.pending.joins[0].clone();
        join.id = "join:new".to_owned();
        join.claimed.name = "dev-box".to_owned();
        join.requested_at_ms = after.boot.server_time_ms;
        after.boot.pending.joins.push(join);

        let found = notices(Some(&before), &after, &RecentUse::default());
        assert_eq!(found.len(), 1, "the old request already notified");
        assert_eq!(found[0].title, "\"dev-box\" wants to join home");
        assert_eq!(
            found[0].body.as_deref(),
            Some("Open kith to check the code and approve.")
        );
        assert_eq!(found[0].target, "/n/home?need=join%3Anew");

        let mut stale = after.clone();
        if let Some(join) = stale.boot.pending.joins.last_mut() {
            join.requested_at_ms = 0;
        }
        assert!(
            notices(Some(&before), &stale, &RecentUse::default()).is_empty(),
            "too old"
        );

        for member in &mut after.boot.status.memberships {
            member.role = Role::Member;
        }
        assert!(
            notices(Some(&before), &after, &RecentUse::default()).is_empty(),
            "members"
        );
    }

    #[test]
    fn the_exit_and_recently_used_devices_going_offline_notify() {
        let before = office_snapshot();
        let mut after = before.clone();
        after.boot.status.exit_state = Some(ExitState::OfflineBlocked);
        let (net, peers) = after.boot.peers.iter_mut().next().expect("a network");
        let net = net.clone();
        let [busy, idle] = [0, 1].map(|index| peers[index].clone());
        assert!(busy.online && idle.online);
        peers[0].online = false;
        peers[1].online = false;

        let now = after.boot.server_time_ms;
        let mut used = RecentUse::default();
        for (at, rx) in [(now - 4000, 10), (now - 2000, 20)] {
            used.tick(&Tick {
                at_ms: at,
                traffic: vec![
                    TrafficTick {
                        network: net.clone(),
                        node: busy.node.clone(),
                        rx,
                        tx: 0,
                    },
                    TrafficTick {
                        network: net.clone(),
                        node: idle.node.clone(),
                        rx: 5,
                        tx: 5,
                    },
                ],
                links: Vec::new(),
            });
        }

        let titles: Vec<String> = notices(Some(&before), &after, &used)
            .into_iter()
            .map(|notice| notice.title)
            .collect();
        assert_eq!(
            titles,
            [
                "Your exit, vps, is offline".to_owned(),
                format!("{} went offline", busy.name)
            ]
        );
    }

    #[test]
    fn the_gate_keeps_notifications_few() {
        let mut gate = Gate::default();
        assert_eq!(
            gate.admit(notice("a"), 0, true),
            None,
            "never while focused"
        );
        for (at, tag) in [(1, "a"), (2, "b"), (3, "c")] {
            assert!(gate.admit(notice(tag), at, false).is_some());
        }
        assert_eq!(gate.admit(notice("a"), 4, false), None, "one per tag");
        assert_eq!(
            gate.admit(notice("d"), 5, false),
            None,
            "the fourth is held"
        );
        assert_eq!(gate.admit(notice("e"), 6, false), None);
        assert_eq!(gate.flush(10, false), None, "the window is still full");
        let merged = gate
            .flush(NOTIFY_COALESCE_WINDOW_MS + 1, false)
            .expect("then one for both");
        assert_eq!(merged.title, "2 things need you in kith");
        assert_eq!(gate.flush(NOTIFY_COALESCE_WINDOW_MS + 2, false), None);
    }

    #[test]
    fn names_cannot_forge_or_flood_a_notification() {
        assert_eq!(shown_name("pixel-8"), "pixel-8");
        assert_eq!(
            shown_name("tv\napproved\u{202e}gpj.exe\u{2066}"),
            "tvapprovedgpj.exe"
        );
        let long = shown_name(&"é".repeat(MAX_NAME * 4));
        assert_eq!(long.chars().count(), MAX_NAME + 1);
        assert!(long.ends_with('…'));
        assert_eq!(shown_name(&"a".repeat(MAX_NAME)), "a".repeat(MAX_NAME));
    }

    #[test]
    fn only_a_few_notifications_wait_at_once() {
        let waiting = Waiting::default();
        let places: Vec<Place> = (0..MAX_WAITING).filter_map(|_| waiting.claim()).collect();
        assert_eq!(places.len(), MAX_WAITING);
        assert!(waiting.claim().is_none(), "the rest show without waiting");
        drop(places);
        assert!(waiting.claim().is_some(), "places come back when dismissed");
    }
}
