//! Join requests waiting for an admin, shared by the control channel and the loop.
//!
//! A request grants nothing: an admin who approves it signs an ordinary `AddDevice` op, so
//! the ledger's rules decide membership as for any other add. Requests are dated by the
//! requester's signature, so every member expires the same request at about the same time
//! and gossip cannot bring an expired one back. Denials stay local to the members and are
//! gossiped only so the request leaves every admin's list.

use core::net::SocketAddr;
use core::time::Duration;
use std::collections::BTreeMap;

use kith_channel::MAX_ADMISSIONS;
use kith_ledger::AdmissionRequest;
use kith_types::{NetworkId, NodeId, Timestamp};

/// How long a request stays pending.
pub(crate) const REQUEST_LIFETIME: Duration = Duration::from_mins(10);

/// How far ahead of this device's clock a request may be dated.
const CLOCK_SKEW: Duration = Duration::from_secs(60);

/// The most requests pending at once in one network.
pub(crate) const MAX_PENDING: usize = 32;

const _: () = assert!(MAX_PENDING <= MAX_ADMISSIONS);

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
    /// device's newer request replaces its older one.
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
            .is_some_and(|at| *at >= request.requested_at)
        {
            return Err(Refusal::Denied);
        }
        let full = network.pending.len() >= MAX_PENDING;
        match network.pending.get_mut(&request.device) {
            Some(held) if held.request.requested_at >= request.requested_at => {
                if held.request == request && held.from.is_none() {
                    held.from = from;
                }
                Ok(false)
            }
            Some(held) => {
                *held = Pending { request, from };
                Ok(true)
            }
            None if full => Err(Refusal::Full),
            None => {
                network
                    .pending
                    .insert(request.device, Pending { request, from });
                Ok(true)
            }
        }
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

fn is_live(requested_at: Timestamp, now: Timestamp) -> bool {
    requested_at <= now.saturating_add(CLOCK_SKEW)
        && now.saturating_duration_since(requested_at) < REQUEST_LIFETIME
}

#[cfg(test)]
mod tests {
    use kith_crypto::SigningKey;
    use kith_ledger::AdmissionRequest;
    use kith_types::{NetworkId, Timestamp};

    use super::{Admissions, MAX_PENDING, REQUEST_LIFETIME, Refusal};

    const NETWORK: NetworkId = NetworkId::from_bytes([7; 32]);

    fn request(seed: u8, at: Timestamp) -> AdmissionRequest {
        AdmissionRequest::create(NETWORK, &SigningKey::from_seed(&[seed; 32]), "laptop", at)
            .unwrap()
    }

    #[test]
    fn requests_expire_are_bounded_and_one_per_device() {
        let start = Timestamp::from_millis(1_000_000_000);
        let mut admissions = Admissions::default();

        assert_eq!(admissions.offer(request(1, start), None, start), Ok(true));
        let later = start.saturating_add(core::time::Duration::from_secs(5));
        assert_eq!(admissions.offer(request(1, later), None, later), Ok(true));
        assert_eq!(
            admissions.offer(request(1, start), None, later),
            Ok(false),
            "an older request does not replace the newer one"
        );
        assert_eq!(admissions.pending(NETWORK, later).len(), 1);

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
    fn a_denied_request_stays_denied_but_the_device_may_ask_again() {
        let start = Timestamp::from_millis(1_000_000_000);
        let asked = request(1, start);
        let mut admissions = Admissions::default();
        admissions.offer(asked.clone(), None, start).unwrap();
        assert!(admissions.deny(NETWORK, asked.device, start).is_some());
        assert_eq!(
            admissions.offer(asked.clone(), None, start),
            Err(Refusal::Denied)
        );

        let mut other_admin = Admissions::default();
        other_admin.offer(asked, None, start).unwrap();
        let (_, denied) = admissions.snapshot(NETWORK, start);
        assert!(other_admin.absorb_denials(NETWORK, denied, start));
        assert!(other_admin.pending(NETWORK, start).is_empty());

        let again = start.saturating_add(core::time::Duration::from_secs(1));
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
