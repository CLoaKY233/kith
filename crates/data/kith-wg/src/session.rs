//! Transport sessions and the previous/current/next rotation (whitepaper §5.4.6, §6).

use core::time::Duration;

use kith_crypto::{Nonce, TAG_LEN, Tag};

use crate::WgError;
use crate::messages::{TRANSPORT_HEADER_LEN, padded_len, write_transport_header};
use crate::noise::TransportKeys;
use crate::replay::ReplayWindow;
use crate::time::{
    REJECT_AFTER_MESSAGES, REJECT_AFTER_TIME, REKEY_AFTER_MESSAGES, REKEY_AFTER_TIME,
};

/// One direction-pair of transport keys with its counters.
pub(crate) struct Keypair {
    keys: TransportKeys,
    pub(crate) local_index: u32,
    remote_index: u32,
    created: Duration,
    pub(crate) initiator: bool,
    send_counter: u64,
    replay: ReplayWindow,
}

impl Keypair {
    pub(crate) fn new(
        keys: TransportKeys,
        local_index: u32,
        remote_index: u32,
        created: Duration,
        initiator: bool,
    ) -> Self {
        Self {
            keys,
            local_index,
            remote_index,
            created,
            initiator,
            send_counter: 0,
            replay: ReplayWindow::new(),
        }
    }

    pub(crate) fn age(&self, now: Duration) -> Duration {
        now.saturating_sub(self.created)
    }

    pub(crate) fn expired(&self, now: Duration) -> bool {
        self.age(now) >= REJECT_AFTER_TIME
    }

    pub(crate) fn can_send(&self, now: Duration) -> bool {
        !self.expired(now) && self.send_counter < REJECT_AFTER_MESSAGES
    }

    /// Only the initiator rekeys on age or volume (§6.2), so the two sides don't race.
    pub(crate) fn needs_rekey(&self, now: Duration) -> bool {
        self.initiator
            && (self.send_counter >= REKEY_AFTER_MESSAGES || self.age(now) >= REKEY_AFTER_TIME)
    }

    /// Writes a transport message carrying `plaintext` into `out`, returning its length.
    pub(crate) fn seal(&mut self, plaintext: &[u8], out: &mut [u8]) -> Result<usize, WgError> {
        let padded = padded_len(plaintext.len());
        let tag_start = TRANSPORT_HEADER_LEN.saturating_add(padded);
        let total = tag_start.saturating_add(TAG_LEN);
        let message = out.get_mut(..total).ok_or(WgError::BufferTooSmall)?;
        let counter = self.send_counter;
        if counter >= REJECT_AFTER_MESSAGES {
            return Err(WgError::SessionExpired);
        }
        write_transport_header(message, self.remote_index, counter)?;
        let (_, rest) = message.split_at_mut(TRANSPORT_HEADER_LEN);
        let (body, tag_slot) = rest.split_at_mut(padded);
        let (data, padding) = body.split_at_mut(plaintext.len());
        data.copy_from_slice(plaintext);
        padding.fill(0);
        let tag = self
            .keys
            .send
            .seal_in_place(Nonce::from_counter(counter), &[], body)
            .map_err(|_| WgError::DecryptionFailed)?;
        tag_slot.copy_from_slice(&tag.to_bytes());
        self.send_counter = counter.saturating_add(1);
        Ok(total)
    }

    /// Authenticates and decrypts a transport payload into `out`, returning the padded
    /// plaintext length. The counter is recorded only after authentication.
    pub(crate) fn open(
        &mut self,
        counter: u64,
        sealed: &[u8],
        out: &mut [u8],
    ) -> Result<usize, WgError> {
        if !self.replay.check(counter) {
            return Err(WgError::Replay);
        }
        let body_len = sealed
            .len()
            .checked_sub(TAG_LEN)
            .ok_or(WgError::MalformedMessage)?;
        let (body, tag) = sealed.split_at(body_len);
        let tag: [u8; TAG_LEN] = tag.try_into().map_err(|_| WgError::MalformedMessage)?;
        let plaintext = out.get_mut(..body_len).ok_or(WgError::BufferTooSmall)?;
        plaintext.copy_from_slice(body);
        if self
            .keys
            .receive
            .open_in_place(
                Nonce::from_counter(counter),
                &[],
                plaintext,
                &Tag::from_bytes(tag),
            )
            .is_err()
        {
            plaintext.fill(0);
            return Err(WgError::DecryptionFailed);
        }
        if !self.replay.accept(counter) {
            return Err(WgError::Replay);
        }
        Ok(body_len)
    }
}

/// Which slot a receiver index matched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Slot {
    Previous,
    Current,
    Next,
}

/// The three keypairs of §6: `next` is a responder session not yet confirmed by the peer.
#[derive(Default)]
pub(crate) struct Keypairs {
    pub(crate) previous: Option<Keypair>,
    pub(crate) current: Option<Keypair>,
    pub(crate) next: Option<Keypair>,
}

impl Keypairs {
    /// Installs a fresh session the way the reference implementations do.
    pub(crate) fn install(&mut self, keypair: Keypair) {
        if keypair.initiator {
            self.previous = match self.next.take() {
                Some(unconfirmed) => Some(unconfirmed),
                None => self.current.take(),
            };
            self.current = Some(keypair);
        } else {
            self.next = Some(keypair);
            self.previous = None;
        }
    }

    /// The first authenticated message on `next` confirms it.
    pub(crate) fn confirm_next(&mut self) {
        if let Some(next) = self.next.take() {
            self.previous = self.current.take();
            self.current = Some(next);
        }
    }

    pub(crate) fn find(&mut self, local_index: u32) -> Option<(Slot, &mut Keypair)> {
        let Self {
            previous,
            current,
            next,
        } = self;
        [
            (Slot::Current, current.as_mut()),
            (Slot::Next, next.as_mut()),
            (Slot::Previous, previous.as_mut()),
        ]
        .into_iter()
        .find_map(|(slot, keypair)| {
            keypair
                .filter(|keypair| keypair.local_index == local_index)
                .map(|keypair| (slot, keypair))
        })
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}
