//! The per-peer state machine: handshakes, sessions and timers (whitepaper §5, §6).

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;

use zeroize::Zeroizing;

use crate::WgError;
use crate::cookie::{CookieChecker, Incoming, Verdict, open_cookie_reply, seal_macs};
use crate::messages::{
    COOKIE_REPLY_LEN, CookieReply, HANDSHAKE_INITIATION_LEN, HANDSHAKE_RESPONSE_LEN,
    INITIATION_MAC1_OFFSET, MAC_LEN, MIN_OUTPUT_LEN, Message, RESPONSE_MAC1_OFFSET, Response,
    Transport, required_output_len,
};
use crate::noise::{
    HalfInitiation, InitiatorHandshake, PeerKeys, StaticIdentity, begin_initiation,
    consume_response, create_initiation, create_response, finish_initiation,
};
use crate::random::{RandomSource, random_u32};
use crate::session::{Keypair, Keypairs, Slot};
use crate::time::{
    COOKIE_LIFETIME, KEEPALIVE_TIMEOUT, Now, REJECT_AFTER_TIME, REKEY_ATTEMPT_TIME, REKEY_TIMEOUT,
    REKEY_TIMEOUT_JITTER_MS, Tai64N, ZERO_KEYS_AFTER,
};

/// At most this many packets wait for a handshake; older ones are dropped first.
pub(crate) const MAX_QUEUED_PACKETS: usize = 16;

const MAX_QUEUED_EVENTS: usize = 8;

/// Handshake initiations per second a standalone peer accepts before demanding cookies.
const STANDALONE_HANDSHAKE_LIMIT: u32 = 20;

/// wireguard-go's per-peer floor between consumed initiations (50 per second).
const MIN_INITIATION_INTERVAL: Duration = Duration::from_millis(20);

/// Whitepaper §6.2: an initiator receiving on an old session rekeys before it expires.
const REKEY_ON_RECEIVE_AGE: Duration = REJECT_AFTER_TIME
    .saturating_sub(KEEPALIVE_TIMEOUT)
    .saturating_sub(REKEY_TIMEOUT);

/// Static configuration of one peer.
pub struct PeerConfig {
    public_key: [u8; 32],
    preshared_key: Option<Zeroizing<[u8; 32]>>,
    persistent_keepalive: Option<Duration>,
}

impl PeerConfig {
    /// A peer identified by its X25519 public key.
    #[must_use]
    pub const fn new(public_key: [u8; 32]) -> Self {
        Self {
            public_key,
            preshared_key: None,
            persistent_keepalive: None,
        }
    }

    /// Mixes a preshared key into every handshake.
    #[must_use]
    pub fn with_preshared_key(mut self, key: [u8; 32]) -> Self {
        self.preshared_key = Some(Zeroizing::new(key));
        self
    }

    /// Sends a keepalive after this many idle seconds; zero disables it.
    #[must_use]
    pub fn with_persistent_keepalive(mut self, seconds: u16) -> Self {
        self.persistent_keepalive = (seconds > 0).then(|| Duration::from_secs(u64::from(seconds)));
        self
    }

    /// The peer's public key.
    #[must_use]
    pub const fn public_key(&self) -> [u8; 32] {
        self.public_key
    }
}

impl fmt::Debug for PeerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerConfig")
            .field(
                "public_key",
                &kith_crypto::AgreementPublic::from_bytes(self.public_key),
            )
            .field(
                "preshared_key",
                &self.preshared_key.as_ref().map(|_| "<redacted>"),
            )
            .field("persistent_keepalive", &self.persistent_keepalive)
            .finish()
    }
}

/// Result of [`Peer::encapsulate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encapsulated {
    /// A transport message of this length is in the output buffer.
    Transport(usize),
    /// No session: the packet was queued and a handshake initiation of this length is in
    /// the output buffer.
    HandshakeInitiation(usize),
    /// No session: the packet was queued; a handshake is already in flight.
    Queued,
}

/// Result of [`Peer::decapsulate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decapsulated {
    /// An IP packet of this length is in the output buffer, for the TUN device.
    Packet(usize),
    /// A datagram of this length is in the output buffer, to send back to the sender.
    /// Afterwards call [`Peer::poll_transmit`] until idle to flush queued packets.
    Reply(usize),
    /// Valid, but nothing to deliver (keepalive or cookie).
    Nothing,
}

/// A change in a peer's session, from [`Peer::poll_event`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PeerEvent {
    /// A new session can send: the first one, or a rekey.
    SessionEstablished,
    /// No session can send any more.
    SessionExpired,
}

/// Result of [`Peer::poll_transmit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transmit {
    /// A datagram of this length is in the output buffer, for the peer's endpoint.
    Send(usize),
    /// Nothing to send until [`Peer::poll_timeout`].
    Idle,
}

struct PendingInitiation {
    handshake: InitiatorHandshake,
    mac1: [u8; MAC_LEN],
}

/// One WireGuard peer: the handshake, the session keypairs and every timer.
pub struct Peer {
    identity: Arc<StaticIdentity>,
    keys: PeerKeys,
    random: Box<dyn RandomSource + Send>,
    checker: CookieChecker,
    index_prefix: u32,
    index_counter: u8,
    persistent_keepalive: Option<Duration>,

    initiation: Option<PendingInitiation>,
    attempt_started: Option<Duration>,
    retransmit_at: Option<Duration>,
    last_initiation_sent: Option<Duration>,
    handshake_wanted: bool,
    last_response: Option<(u32, [u8; MAC_LEN])>,
    cookie: Option<([u8; MAC_LEN], Duration)>,
    greatest_timestamp: Option<Tai64N>,
    last_initiation_consumed: Option<Duration>,

    keypairs: Keypairs,
    last_session_at: Option<Duration>,
    queue: VecDeque<Vec<u8>>,
    established: bool,
    last_handshake: Option<Duration>,
    events: VecDeque<PeerEvent>,
    rx_bytes: u64,
    tx_bytes: u64,

    last_sent: Option<Duration>,
    last_received: Option<Duration>,
    keepalive_due: Option<Duration>,
    new_handshake_due: Option<Duration>,
}

impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Peer")
            .field(
                "public_key",
                &kith_crypto::AgreementPublic::from_bytes(self.keys.public),
            )
            .field("has_session", &self.keypairs.current.is_some())
            .field("handshake_in_flight", &self.initiation.is_some())
            .field("queued", &self.queue.len())
            .finish_non_exhaustive()
    }
}

impl Peer {
    /// A standalone peer with its own cookie checker and a random index prefix.
    pub fn new(
        identity: Arc<StaticIdentity>,
        config: &PeerConfig,
        mut random: Box<dyn RandomSource + Send>,
    ) -> Result<Self, WgError> {
        let prefix = random_u32(random.as_mut())? & !0xff;
        Self::with_index_prefix(identity, config, random, prefix)
    }

    /// Session indices are `prefix | counter`, so a device routes on the upper 24 bits.
    pub(crate) fn with_index_prefix(
        identity: Arc<StaticIdentity>,
        config: &PeerConfig,
        mut random: Box<dyn RandomSource + Send>,
        index_prefix: u32,
    ) -> Result<Self, WgError> {
        let keys = PeerKeys::new(
            &identity,
            config.public_key,
            config.preshared_key.as_deref(),
        )?;
        let index_counter = random_u32(random.as_mut())?.to_le_bytes()[0];
        Ok(Self {
            identity,
            keys,
            random,
            checker: CookieChecker::new(STANDALONE_HANDSHAKE_LIMIT),
            index_prefix,
            index_counter,
            persistent_keepalive: config.persistent_keepalive,
            initiation: None,
            attempt_started: None,
            retransmit_at: None,
            last_initiation_sent: None,
            handshake_wanted: false,
            last_response: None,
            cookie: None,
            greatest_timestamp: None,
            last_initiation_consumed: None,
            keypairs: Keypairs::default(),
            last_session_at: None,
            queue: VecDeque::new(),
            established: false,
            last_handshake: None,
            events: VecDeque::with_capacity(MAX_QUEUED_EVENTS),
            rx_bytes: 0,
            tx_bytes: 0,
            last_sent: None,
            last_received: None,
            keepalive_due: None,
            new_handshake_due: None,
        })
    }

    /// The peer's public key.
    #[must_use]
    pub const fn public_key(&self) -> [u8; 32] {
        self.keys.public
    }

    /// Starts a new handshake as soon as the rate limit allows, as after the peer restarted.
    /// A handshake already in flight is retransmitted now, since its path may have changed.
    pub fn request_handshake(&mut self) {
        self.handshake_wanted = true;
        if self.initiation.is_some() {
            self.retransmit_at = Some(Duration::ZERO);
        }
    }

    /// Whether a session usable for sending exists at `now`.
    #[must_use]
    pub fn has_session(&self, now: Duration) -> bool {
        self.keypairs
            .current
            .as_ref()
            .is_some_and(|keypair| keypair.can_send(now))
    }

    /// When the latest session was established, in monotonic time.
    #[must_use]
    pub const fn last_handshake(&self) -> Option<Duration> {
        self.last_handshake
    }

    /// When an authenticated handshake or transport message last arrived from the peer,
    /// keepalives included, in monotonic time.
    #[must_use]
    pub const fn last_received(&self) -> Option<Duration> {
        self.last_received
    }

    /// Bytes of authenticated datagrams received from the peer.
    #[must_use]
    pub const fn rx_bytes(&self) -> u64 {
        self.rx_bytes
    }

    /// Bytes of datagrams produced for the peer.
    #[must_use]
    pub const fn tx_bytes(&self) -> u64 {
        self.tx_bytes
    }

    /// The next session change, oldest first. Poll after every call that takes `now`.
    pub fn poll_event(&mut self) -> Option<PeerEvent> {
        self.events.pop_front()
    }

    /// Encrypts an IP packet for the peer into `out`.
    ///
    /// `out` must hold [`required_output_len`]`(plaintext.len())` bytes.
    pub fn encapsulate(
        &mut self,
        now: Now,
        plaintext: &[u8],
        out: &mut [u8],
    ) -> Result<Encapsulated, WgError> {
        if out.len() < required_output_len(plaintext.len()) {
            return Err(WgError::BufferTooSmall);
        }
        let at = now.monotonic;
        if let Some(keypair) = self
            .keypairs
            .current
            .as_mut()
            .filter(|keypair| keypair.can_send(at))
        {
            let len = keypair.seal(plaintext, out)?;
            self.handshake_wanted |= keypair.needs_rekey(at);
            self.on_sent(at, !plaintext.is_empty());
            self.count_sent(len);
            return Ok(Encapsulated::Transport(len));
        }

        self.track_expiry(at);
        if self.queue.len() >= MAX_QUEUED_PACKETS {
            self.queue.pop_front();
        }
        self.queue.push_back(plaintext.to_vec());
        if self.may_initiate(at) {
            return self
                .send_initiation(now, out)
                .map(Encapsulated::HandshakeInitiation);
        }
        Ok(Encapsulated::Queued)
    }

    /// Processes one datagram from the peer, with no source address for cookies.
    pub fn decapsulate(
        &mut self,
        now: Now,
        datagram: &[u8],
        out: &mut [u8],
    ) -> Result<Decapsulated, WgError> {
        self.decapsulate_from(now, &[], datagram, out)
    }

    /// Processes one datagram; `source` encodes the sender's address for cookie replies.
    ///
    /// `out` must hold at least [`required_output_len`]`(0)` and the datagram's length.
    pub fn decapsulate_from(
        &mut self,
        now: Now,
        source: &[u8],
        datagram: &[u8],
        out: &mut [u8],
    ) -> Result<Decapsulated, WgError> {
        if out.len() < MIN_OUTPUT_LEN.max(datagram.len()) {
            return Err(WgError::BufferTooSmall);
        }
        match Message::parse(datagram)? {
            Message::Initiation(message) => {
                if let Some(reply) = verify_handshake(
                    &mut self.checker,
                    self.random.as_mut(),
                    &self.identity,
                    now.monotonic,
                    &Incoming {
                        source,
                        datagram,
                        mac1_offset: INITIATION_MAC1_OFFSET,
                        sender: message.sender,
                    },
                    out,
                )? {
                    return Ok(reply);
                }
                let half = begin_initiation(&self.identity, &message)?;
                if half.peer_public != self.keys.public {
                    return Err(WgError::UnknownPeer);
                }
                self.handle_initiation(now, half, out)
            }
            Message::Response(message) => {
                if let Some(reply) = verify_handshake(
                    &mut self.checker,
                    self.random.as_mut(),
                    &self.identity,
                    now.monotonic,
                    &Incoming {
                        source,
                        datagram,
                        mac1_offset: RESPONSE_MAC1_OFFSET,
                        sender: message.sender,
                    },
                    out,
                )? {
                    return Ok(reply);
                }
                self.handle_response(now, &message, out)
            }
            Message::CookieReply(reply) => self.handle_cookie_reply(now, &reply),
            Message::Transport(message) => self.handle_transport(now, &message, out),
        }
    }

    /// Emits the next due datagram (retransmission, rekey, queued packet or keepalive).
    ///
    /// Call until it returns [`Transmit::Idle`], then again at [`Peer::poll_timeout`].
    pub fn poll_transmit(&mut self, now: Now, out: &mut [u8]) -> Result<Transmit, WgError> {
        if out.len() < MIN_OUTPUT_LEN {
            return Err(WgError::BufferTooSmall);
        }
        let at = now.monotonic;

        if self
            .last_session_at
            .is_some_and(|made| at >= made.saturating_add(ZERO_KEYS_AFTER))
        {
            self.keypairs.clear();
            self.initiation = None;
            self.last_response = None;
            self.last_session_at = None;
        }
        self.track_expiry(at);

        if self
            .attempt_started
            .is_some_and(|started| at >= started.saturating_add(REKEY_ATTEMPT_TIME))
        {
            self.initiation = None;
            self.attempt_started = None;
            self.retransmit_at = None;
            self.handshake_wanted = false;
            self.new_handshake_due = None;
            self.queue.clear();
        }

        if self.initiation.is_some() {
            if self.retransmit_at.is_some_and(|due| at >= due) {
                return self.send_initiation(now, out).map(Transmit::Send);
            }
            self.new_handshake_due = None;
        } else if self.wants_handshake(at) && self.may_initiate(at) {
            return self.send_initiation(now, out).map(Transmit::Send);
        }

        let sendable = self.has_session(at);
        if sendable && let Some(packet) = self.queue.pop_front() {
            return self.send_on_current(at, &packet, out).map(Transmit::Send);
        }

        if self.keepalive_due.is_some_and(|due| at >= due) {
            self.keepalive_due = None;
            if sendable {
                return self.send_on_current(at, &[], out).map(Transmit::Send);
            }
        }

        if sendable && self.persistent_keepalive_due().is_some_and(|due| at >= due) {
            return self.send_on_current(at, &[], out).map(Transmit::Send);
        }

        Ok(Transmit::Idle)
    }

    /// The earliest monotonic time at which [`Peer::poll_transmit`] may have work.
    #[must_use]
    pub fn poll_timeout(&self) -> Option<Duration> {
        let initiate_at = self
            .last_initiation_sent
            .map_or(Duration::ZERO, |sent| sent.saturating_add(REKEY_TIMEOUT));
        let has_session = self.keypairs.current.is_some();
        let mut handshake_due = None;
        if self.initiation.is_none() {
            if self.handshake_wanted || (!has_session && !self.queue.is_empty()) {
                handshake_due = Some(Duration::ZERO);
            }
            handshake_due = earliest(handshake_due, self.new_handshake_due);
            if !has_session {
                handshake_due = earliest(handshake_due, self.persistent_keepalive_due());
            }
        }
        let flush_due = (has_session && !self.queue.is_empty()).then_some(Duration::ZERO);
        let expiry_due = self
            .keypairs
            .current
            .as_ref()
            .filter(|_| self.established)
            .map(Keypair::expires_at);
        let keepalive_due = if has_session {
            earliest(self.keepalive_due, self.persistent_keepalive_due())
        } else {
            None
        };
        [
            handshake_due.map(|due| due.max(initiate_at)),
            self.initiation.as_ref().and(self.retransmit_at),
            self.attempt_started
                .map(|started| started.saturating_add(REKEY_ATTEMPT_TIME)),
            self.last_session_at
                .map(|made| made.saturating_add(ZERO_KEYS_AFTER)),
            flush_due,
            keepalive_due,
            expiry_due,
        ]
        .into_iter()
        .fold(None, earliest)
    }

    pub(crate) fn local_index_prefix(&self) -> u32 {
        self.index_prefix
    }

    pub(crate) fn handle_initiation(
        &mut self,
        now: Now,
        half: HalfInitiation,
        out: &mut [u8],
    ) -> Result<Decapsulated, WgError> {
        let (timestamp, handshake) = finish_initiation(half, &self.keys)?;
        if self
            .greatest_timestamp
            .is_some_and(|greatest| timestamp <= greatest)
        {
            return Err(WgError::Replay);
        }
        let at = now.monotonic;
        if self
            .last_initiation_consumed
            .is_some_and(|last| at.saturating_sub(last) < MIN_INITIATION_INTERVAL)
        {
            return Err(WgError::Replay);
        }
        let slot = out
            .get_mut(..HANDSHAKE_RESPONSE_LEN)
            .ok_or(WgError::BufferTooSmall)?;
        let local_index = self.allocate_index();
        let remote_index = handshake.remote_index;
        let (response, keys) =
            create_response(handshake, &self.keys, local_index, self.random.as_mut())?;
        let mut bytes = response.to_bytes();
        let mac1 = seal_macs(
            &mut bytes,
            RESPONSE_MAC1_OFFSET,
            &self.keys.mac1_key,
            self.valid_cookie(at),
        )?;
        slot.copy_from_slice(&bytes);

        self.greatest_timestamp = Some(timestamp);
        self.last_initiation_consumed = Some(at);
        self.keypairs
            .install(Keypair::new(keys, local_index, remote_index, at, false));
        self.last_response = Some((local_index, mac1));
        self.last_session_at = Some(at);
        self.on_received(at);
        self.on_sent(at, false);
        self.count_received(HANDSHAKE_INITIATION_LEN);
        self.count_sent(HANDSHAKE_RESPONSE_LEN);
        Ok(Decapsulated::Reply(HANDSHAKE_RESPONSE_LEN))
    }

    pub(crate) fn handle_response(
        &mut self,
        now: Now,
        message: &Response,
        out: &mut [u8],
    ) -> Result<Decapsulated, WgError> {
        let pending = self
            .initiation
            .as_ref()
            .filter(|pending| pending.handshake.local_index == message.receiver)
            .ok_or(WgError::UnknownIndex)?;
        let keys = consume_response(&pending.handshake, &self.identity, &self.keys, message)?;
        let at = now.monotonic;
        self.initiation = None;
        self.attempt_started = None;
        self.retransmit_at = None;
        self.handshake_wanted = false;
        self.track_expiry(at);
        self.keypairs.install(Keypair::new(
            keys,
            message.receiver,
            message.sender,
            at,
            true,
        ));
        self.last_session_at = Some(at);
        self.on_received(at);
        self.count_received(HANDSHAKE_RESPONSE_LEN);
        self.on_established(at);

        // Whitepaper §5.4.3: the initiator must send first so the responder can confirm.
        let first = self.queue.pop_front().unwrap_or_default();
        self.send_on_current(at, &first, out)
            .map(Decapsulated::Reply)
    }

    pub(crate) fn handle_cookie_reply(
        &mut self,
        now: Now,
        reply: &CookieReply,
    ) -> Result<Decapsulated, WgError> {
        let sent_mac1 = match (&self.initiation, self.last_response) {
            (Some(pending), _) if pending.handshake.local_index == reply.receiver => pending.mac1,
            (_, Some((index, mac1))) if index == reply.receiver => mac1,
            _ => return Err(WgError::UnknownIndex),
        };
        let cookie = open_cookie_reply(reply, &self.keys.cookie_key, &sent_mac1)?;
        self.cookie = Some((cookie, now.monotonic));
        self.count_received(COOKIE_REPLY_LEN);
        Ok(Decapsulated::Nothing)
    }

    pub(crate) fn handle_transport(
        &mut self,
        now: Now,
        message: &Transport<'_>,
        out: &mut [u8],
    ) -> Result<Decapsulated, WgError> {
        let at = now.monotonic;
        let (slot, keypair) = self
            .keypairs
            .find(message.receiver)
            .ok_or(WgError::UnknownIndex)?;
        if keypair.expired(at) {
            return Err(WgError::SessionExpired);
        }
        let padded = keypair.open(message.counter, message.sealed, out)?;
        if slot == Slot::Next {
            self.track_expiry(at);
            self.keypairs.confirm_next();
            self.on_established(at);
        }
        if slot == Slot::Current && keypair_is_old_initiator(self.keypairs.current.as_ref(), at) {
            self.handshake_wanted = true;
        }
        self.on_received(at);
        self.count_received(message.datagram_len());
        if padded == 0 {
            return Ok(Decapsulated::Nothing);
        }
        let len = inner_packet_len(out.get(..padded).ok_or(WgError::InvalidPacket)?)?;
        if self.keepalive_due.is_none() {
            self.keepalive_due = Some(at.saturating_add(KEEPALIVE_TIMEOUT));
        }
        Ok(Decapsulated::Packet(len))
    }

    fn send_initiation(&mut self, now: Now, out: &mut [u8]) -> Result<usize, WgError> {
        let slot = out
            .get_mut(..HANDSHAKE_INITIATION_LEN)
            .ok_or(WgError::BufferTooSmall)?;
        let at = now.monotonic;
        let local_index = self.allocate_index();
        let (message, handshake) = create_initiation(
            &self.identity,
            &self.keys,
            local_index,
            now.wall,
            self.random.as_mut(),
        )?;
        let mut bytes = message.to_bytes();
        let mac1 = seal_macs(
            &mut bytes,
            INITIATION_MAC1_OFFSET,
            &self.keys.mac1_key,
            self.valid_cookie(at),
        )?;
        let jitter_ms = random_u32(self.random.as_mut())? % REKEY_TIMEOUT_JITTER_MS;
        slot.copy_from_slice(&bytes);

        self.initiation = Some(PendingInitiation { handshake, mac1 });
        self.attempt_started.get_or_insert(at);
        self.retransmit_at = Some(
            at.saturating_add(REKEY_TIMEOUT)
                .saturating_add(Duration::from_millis(u64::from(jitter_ms))),
        );
        self.last_initiation_sent = Some(at);
        self.handshake_wanted = false;
        self.new_handshake_due = None;
        self.on_sent(at, false);
        self.count_sent(HANDSHAKE_INITIATION_LEN);
        Ok(HANDSHAKE_INITIATION_LEN)
    }

    fn send_on_current(
        &mut self,
        at: Duration,
        plaintext: &[u8],
        out: &mut [u8],
    ) -> Result<usize, WgError> {
        let keypair = self
            .keypairs
            .current
            .as_mut()
            .filter(|keypair| keypair.can_send(at))
            .ok_or(WgError::SessionExpired)?;
        let len = keypair.seal(plaintext, out)?;
        self.handshake_wanted |= keypair.needs_rekey(at);
        self.on_sent(at, !plaintext.is_empty());
        self.count_sent(len);
        Ok(len)
    }

    fn wants_handshake(&self, at: Duration) -> bool {
        let session = self.has_session(at);
        self.handshake_wanted
            || (!session && !self.queue.is_empty())
            || self.new_handshake_due.is_some_and(|due| at >= due)
            || (!session && self.persistent_keepalive_due().is_some_and(|due| at >= due))
    }

    fn may_initiate(&self, at: Duration) -> bool {
        self.initiation.is_none()
            && self
                .last_initiation_sent
                .is_none_or(|sent| at >= sent.saturating_add(REKEY_TIMEOUT))
    }

    fn persistent_keepalive_due(&self) -> Option<Duration> {
        let interval = self.persistent_keepalive?;
        let last = latest_activity(self.last_sent, self.last_received);
        Some(last.saturating_add(interval))
    }

    fn valid_cookie(&self, at: Duration) -> Option<&[u8; MAC_LEN]> {
        self.cookie
            .as_ref()
            .filter(|(_, received)| at.saturating_sub(*received) < COOKIE_LIFETIME)
            .map(|(cookie, _)| cookie)
    }

    fn allocate_index(&mut self) -> u32 {
        self.index_counter = self.index_counter.wrapping_add(1);
        self.index_prefix | u32::from(self.index_counter)
    }

    fn on_sent(&mut self, at: Duration, data: bool) {
        self.last_sent = Some(at);
        self.keepalive_due = None;
        if data && self.new_handshake_due.is_none() {
            self.new_handshake_due = Some(
                at.saturating_add(KEEPALIVE_TIMEOUT)
                    .saturating_add(REKEY_TIMEOUT),
            );
        }
    }

    fn on_received(&mut self, at: Duration) {
        self.last_received = Some(at);
        self.new_handshake_due = None;
    }

    fn count_sent(&mut self, len: usize) {
        self.tx_bytes = self.tx_bytes.saturating_add(byte_count(len));
    }

    fn count_received(&mut self, len: usize) {
        self.rx_bytes = self.rx_bytes.saturating_add(byte_count(len));
    }

    fn track_expiry(&mut self, at: Duration) {
        if self.established && !self.has_session(at) {
            self.established = false;
            self.keypairs.retire_current();
            self.push_event(PeerEvent::SessionExpired);
        }
    }

    fn on_established(&mut self, at: Duration) {
        self.established = true;
        self.last_handshake = Some(at);
        self.push_event(PeerEvent::SessionEstablished);
    }

    fn push_event(&mut self, event: PeerEvent) {
        if self.events.len() >= MAX_QUEUED_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

/// Checks MACs of a handshake message; `Some` carries a cookie reply already in `out`.
pub(crate) fn verify_handshake(
    checker: &mut CookieChecker,
    random: &mut dyn RandomSource,
    identity: &StaticIdentity,
    at: Duration,
    incoming: &Incoming<'_>,
    out: &mut [u8],
) -> Result<Option<Decapsulated>, WgError> {
    match checker.verify(at, random, identity, incoming)? {
        Verdict::Process => Ok(None),
        Verdict::SendCookie(reply) => {
            let bytes = reply.to_bytes();
            out.get_mut(..bytes.len())
                .ok_or(WgError::BufferTooSmall)?
                .copy_from_slice(&bytes);
            Ok(Some(Decapsulated::Reply(bytes.len())))
        }
    }
}

fn keypair_is_old_initiator(keypair: Option<&Keypair>, at: Duration) -> bool {
    keypair.is_some_and(|keypair| keypair.initiator && keypair.age(at) >= REKEY_ON_RECEIVE_AGE)
}

fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

fn byte_count(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

fn latest_activity(sent: Option<Duration>, received: Option<Duration>) -> Duration {
    sent.max(received).unwrap_or(Duration::ZERO)
}

/// The IP packet's own length, which strips the zero padding added by the sender.
fn inner_packet_len(plaintext: &[u8]) -> Result<usize, WgError> {
    let read_u16 = |offset: usize| {
        plaintext
            .get(offset..offset.saturating_add(2))
            .and_then(|bytes| bytes.try_into().ok())
            .map(u16::from_be_bytes)
            .map(usize::from)
            .ok_or(WgError::InvalidPacket)
    };
    let len = match plaintext.first().map(|byte| byte >> 4) {
        Some(4) if plaintext.len() >= 20 => read_u16(2)?,
        Some(6) if plaintext.len() >= 40 => read_u16(4)?.saturating_add(40),
        _ => return Err(WgError::InvalidPacket),
    };
    if len == 0 || len > plaintext.len() {
        return Err(WgError::InvalidPacket);
    }
    Ok(len)
}
