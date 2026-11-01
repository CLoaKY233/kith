//! Join requests waiting for an admin, shared by the control channel and the loop.
//!
//! A request grants nothing: an admin who approves it signs an ordinary `AddDevice` op, so
//! the ledger's rules decide membership as for any other add. Requests are dated by the
//! requester's signature, so every member expires the same request at about the same time
//! and gossip cannot bring an expired one back. Denials stay local to the members and are
//! gossiped only so the request leaves every admin's list.

use core::net::{IpAddr, SocketAddr};
use core::time::Duration;
use std::collections::BTreeMap;

use kith_channel::MAX_ADMISSIONS;
use kith_ledger::AdmissionRequest;
use kith_types::{NetworkId, NodeId, Timestamp};

/// How long a request stays pending.
pub(crate) const REQUEST_LIFETIME: Duration = Duration::from_mins(10);

/// How old a pending request must be before the same device may replace it, and how long
/// a denial keeps the device from asking again. Both stop one device flooding admins.
pub(crate) const REFRESH_AFTER: Duration = Duration::from_mins(5);

/// How far ahead of this device's clock a request may be dated.
const CLOCK_SKEW: Duration = Duration::from_secs(60);

/// The most requests pending at once in one network.
pub(crate) const MAX_PENDING: usize = 32;

/// The most requests pending at once in one network that reached this device from one
/// address, or one IPv6 /64.
pub(crate) const MAX_PENDING_PER_SOURCE: usize = 4;

const _: () = assert!(MAX_PENDING <= MAX_ADMISSIONS);
const _: () = assert!(REFRESH_AFTER.as_millis() < REQUEST_LIFETIME.as_millis());

/// Why a request was not held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refusal {
    /// The signature or the name is invalid.
    #[error("the request is not validly signed")]
    BadSignature,
    /// It is older than [`REQUEST_LIFETIME`] or dated in the future.
    #[error("the request expired; ask again")]
    Expired,
    /// An admin denied it.
    #[error("denied")]
    Denied,
    /// [`MAX_PENDING`] other requests are waiting.
    #[error("too many devices are waiting for approval; try again later")]
    Full,
    /// [`MAX_PENDING_PER_SOURCE`] other requests from the same address are waiting.
    #[error("too many requests from this address are waiting; try again later")]
    Busy,
}

/// One pending request and where it came from, if it reached this device directly.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    /// The signed request.
    pub(crate) request: AdmissionRequest,
    /// The requester's address, when it asked this device.
    pub(crate) from: Option<SocketAddr>,
}

#[derive(Debug, Default)]
struct NetworkAdmissions {
    pending: BTreeMap<NodeId, Pending>,
    /// Denied requests, by device and the request's date.
    denied: BTreeMap<NodeId, Timestamp>,
}

impl NetworkAdmissions {
    /// Records a denial, dropping the oldest one so a gossip message can always carry them.
    fn remember_denial(&mut self, device: NodeId, at: Timestamp) {
        self.denied.insert(device, at);
        while self.denied.len() > MAX_ADMISSIONS {
            let oldest = self
                .denied
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(device, _)| *device);
            match oldest {
                Some(device) => self.denied.remove(&device),
                None => break,
            };
        }
    }
}

/// Pending and denied join requests of every network.
#[derive(Debug, Default)]
pub(crate) struct Admissions {
    networks: BTreeMap<NetworkId, NetworkAdmissions>,
}

impl Admissions {
    /// Holds `request` pending. Returns whether it is new, so it is worth announcing; a
    /// device's newer request replaces its older one once that is [`REFRESH_AFTER`] old.
    pub(crate) fn offer(
        &mut self,
        request: AdmissionRequest,
        from: Option<SocketAddr>,
        now: Timestamp,
    ) -> Result<bool, Refusal> {
        if !is_live(request.requested_at, now) {
            return Err(Refusal::Expired);
        }
        request.verify().map_err(|_| Refusal::BadSignature)?;
        self.prune(now);
        let network = self.networks.entry(request.network).or_default();
        if network
            .denied
            .get(&request.device)
            .is_some_and(|at| request.requested_at < at.saturating_add(REFRESH_AFTER))
        {
            return Err(Refusal::Denied);
        }
        if let Some(held) = network.pending.get_mut(&request.device) {
            if request.requested_at < held.request.requested_at.saturating_add(REFRESH_AFTER) {
                if held.request == request && held.from.is_none() {
                    held.from = from;
                }
                return Ok(false);
            }
            *held = Pending {
                request,
                from: from.or(held.from),
            };
            return Ok(true);
        }
        if network.pending.len() >= MAX_PENDING {
            return Err(Refusal::Full);
        }
        if let Some(source) = from.map(|addr| source(addr.ip())) {
            let same = network
                .pending
                .values()
                .filter_map(|held| held.from)
                .filter(|addr| self::source(addr.ip()) == source)
                .count();
            if same >= MAX_PENDING_PER_SOURCE {
                return Err(Refusal::Busy);
            }
        }
        network
            .pending
            .insert(request.device, Pending { request, from });
        Ok(true)
    }

    /// Denies `device`'s pending request in `network` and returns it.
    pub(crate) fn deny(
        &mut self,
        network: NetworkId,
        device: NodeId,
        now: Timestamp,
    ) -> Option<AdmissionRequest> {
        self.prune(now);
        let admissions = self.networks.get_mut(&network)?;
        let pending = admissions.pending.remove(&device)?;
        admissions.remember_denial(device, pending.request.requested_at);
        Some(pending.request)
    }

    /// Records denials another admin gossiped. Returns whether anything changed.
    pub(crate) fn absorb_denials(
        &mut self,
        network: NetworkId,
        denied: Vec<(NodeId, Timestamp)>,
        now: Timestamp,
    ) -> bool {
        let admissions = self.networks.entry(network).or_default();
        let mut changed = false;
        for (device, at) in denied {
            if !is_live(at, now) || admissions.denied.get(&device).is_some_and(|d| *d >= at) {
                continue;
            }
            admissions.remember_denial(device, at);
            if admissions
                .pending
                .get(&device)
                .is_some_and(|held| held.request.requested_at <= at)
            {
                admissions.pending.remove(&device);
            }
            changed = true;
        }
        changed
    }

    /// Removes `device`'s pending request in `network`, once it is approved or a member.
    pub(crate) fn take(&mut self, network: NetworkId, device: NodeId) -> Option<Pending> {
        self.networks.get_mut(&network)?.pending.remove(&device)
    }

    /// The live pending requests of `network`, oldest first.
    pub(crate) fn pending(&mut self, network: NetworkId, now: Timestamp) -> Vec<Pending> {
        self.prune(now);
        let mut pending: Vec<Pending> = self
            .networks
            .get(&network)
            .map(|admissions| admissions.pending.values().cloned().collect())
            .unwrap_or_default();
        pending.sort_by_key(|held| held.request.requested_at);
        pending
    }

    /// What to gossip for `network`: its pending requests and denials.
    pub(crate) fn snapshot(
        &mut self,
        network: NetworkId,
        now: Timestamp,
    ) -> (Vec<AdmissionRequest>, Vec<(NodeId, Timestamp)>) {
        self.prune(now);
        self.networks
            .get(&network)
            .map(|admissions| {
                (
                    admissions
                        .pending
                        .values()
                        .map(|held| held.request.clone())
                        .collect(),
                    admissions
                        .denied
                        .iter()
                        .map(|(device, at)| (*device, *at))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    /// Forgets everything about `network`.
    pub(crate) fn forget(&mut self, network: NetworkId) {
        self.networks.remove(&network);
    }

    fn prune(&mut self, now: Timestamp) {
        self.networks.retain(|_, admissions| {
            admissions
                .pending
                .retain(|_, held| is_live(held.request.requested_at, now));
            admissions.denied.retain(|_, at| is_live(*at, now));
            !admissions.pending.is_empty() || !admissions.denied.is_empty()
        });
    }
}

/// The address a request came from, widened to its /64 for IPv6, where one host holds many.
fn source(ip: IpAddr) -> u128 {
    match ip.to_canonical() {
        IpAddr::V4(v4) => u128::from(v4.to_bits()),
        IpAddr::V6(v6) => (v6.to_bits() >> 64) | (1 << 64),
    }
}

fn is_live(requested_at: Timestamp, now: Timestamp) -> bool {
    requested_at <= now.saturating_add(CLOCK_SKEW)
        && now.saturating_duration_since(requested_at) < REQUEST_LIFETIME
}

#[cfg(test)]
mod tests {
    use core::net::SocketAddr;
    use core::time::Duration;

    use kith_crypto::SigningKey;
    use kith_ledger::AdmissionRequest;
    use kith_types::{NetworkId, Timestamp};

    use super::{
        Admissions, MAX_PENDING, MAX_PENDING_PER_SOURCE, REFRESH_AFTER, REQUEST_LIFETIME, Refusal,
    };

    const NETWORK: NetworkId = NetworkId::from_bytes([7; 32]);

    fn request(seed: u8, at: Timestamp) -> AdmissionRequest {
        AdmissionRequest::create(NETWORK, &SigningKey::from_seed(&[seed; 32]), "laptop", at)
            .unwrap()
    }

    fn addr(text: &str) -> Option<SocketAddr> {
        text.parse().ok()
    }

    #[test]
    fn requests_expire_are_bounded_and_one_per_device() {
        let start = Timestamp::from_millis(1_000_000_000);
        let mut admissions = Admissions::default();

        assert_eq!(admissions.offer(request(1, start), None, start), Ok(true));
        let soon = start.saturating_add(Duration::from_secs(5));
        assert_eq!(
            admissions.offer(request(1, soon), None, soon),
            Ok(false),
            "a device cannot flood admins by re-signing its request"
        );
        let later = start.saturating_add(REFRESH_AFTER);
        assert_eq!(admissions.offer(request(1, later), None, later), Ok(true));
        assert_eq!(
            admissions.offer(request(1, start), None, later),
            Ok(false),
            "an older request does not replace the newer one"
        );
        let pending = admissions.pending(NETWORK, later);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request.requested_at, later);

        for seed in 2..=u8::try_from(MAX_PENDING).unwrap() {
            assert_eq!(
                admissions.offer(request(seed, later), None, later),
                Ok(true)
            );
        }
        assert_eq!(
            admissions.offer(request(200, later), None, later),
            Err(Refusal::Full)
        );

        let mut forged = request(201, later);
        forged.name = String::from("someone-else");
        assert_eq!(
            admissions.offer(forged, None, later),
            Err(Refusal::BadSignature)
        );
        let future = later.saturating_add(Duration::from_mins(2));
        assert_eq!(
            admissions.offer(request(202, future), None, later),
            Err(Refusal::Expired),
            "a request dated in the future would outlive its lifetime"
        );

        let expired = later.saturating_add(REQUEST_LIFETIME);
        assert!(admissions.pending(NETWORK, expired).is_empty());
        assert_eq!(
            admissions.offer(request(1, later), None, expired),
            Err(Refusal::Expired),
            "gossip cannot bring an expired request back"
        );
        assert_eq!(
            admissions.offer(request(200, expired), None, expired),
            Ok(true)
        );
    }

    #[test]
    fn one_address_cannot_fill_the_queue() {
        let at = Timestamp::from_millis(1_000_000_000);
        let mut admissions = Admissions::default();
        let mut seed = 1;
        for port in 0..MAX_PENDING_PER_SOURCE {
            let from = addr(&format!("[2001:db8::{port}]:{}", 1000 + port));
            assert_eq!(admissions.offer(request(seed, at), from, at), Ok(true));
            seed += 1;
        }
        assert_eq!(
            admissions.offer(request(seed, at), addr("[2001:db8::ffff]:1"), at),
            Err(Refusal::Busy),
            "addresses in one /64 count together"
        );
        assert_eq!(
            admissions.offer(request(seed, at), addr("[2001:db8:0:1::1]:1"), at),
            Ok(true)
        );
        assert_eq!(
            admissions.offer(request(seed + 1, at), addr("192.0.2.1:1"), at),
            Ok(true)
        );
        assert_eq!(
            admissions.offer(request(seed + 2, at), None, at),
            Ok(true),
            "gossiped requests are bounded by the network-wide cap"
        );
    }

    #[test]
    fn a_denial_holds_the_device_off_for_a_while_and_spreads_from_admins() {
        let start = Timestamp::from_millis(1_000_000_000);
        let asked = request(1, start);
        let mut admissions = Admissions::default();
        admissions.offer(asked.clone(), None, start).unwrap();
        assert!(admissions.deny(NETWORK, asked.device, start).is_some());
        assert_eq!(
            admissions.offer(asked.clone(), None, start),
            Err(Refusal::Denied)
        );
        let soon = start.saturating_add(Duration::from_secs(1));
        assert_eq!(
            admissions.offer(request(1, soon), None, soon),
            Err(Refusal::Denied),
            "a denied device cannot ask again straight away"
        );

        let mut other_admin = Admissions::default();
        other_admin.offer(asked, None, start).unwrap();
        let (_, denied) = admissions.snapshot(NETWORK, start);
        assert!(other_admin.absorb_denials(NETWORK, denied.clone(), start));
        assert!(!other_admin.absorb_denials(NETWORK, denied, start));
        assert!(other_admin.pending(NETWORK, start).is_empty());

        let again = start.saturating_add(REFRESH_AFTER);
        assert_eq!(admissions.offer(request(1, again), None, again), Ok(true));

        for seed in 2..=200 {
            admissions.offer(request(seed, again), None, again).ok();
            admissions.deny(NETWORK, request(seed, again).device, again);
        }
        let (_, denied) = admissions.snapshot(NETWORK, again);
        assert!(
            denied.len() <= super::MAX_ADMISSIONS,
            "denials must fit in one gossip message"
        );
    }
}
