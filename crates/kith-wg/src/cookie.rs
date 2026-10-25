//! MAC1/MAC2 and cookie replies (whitepaper §5.3, §5.4.4, §5.4.7).

use core::fmt;
use core::time::Duration;

use kith_crypto::Tag;
use kith_crypto::wireguard::{CookieAeadKey, constant_time_eq, mac};
use zeroize::Zeroizing;

use crate::WgError;
use crate::messages::{CookieReply, MAC_LEN};
use crate::noise::StaticIdentity;
use crate::random::RandomSource;
use crate::time::COOKIE_SECRET_LIFETIME;

const LOAD_WINDOW: Duration = Duration::from_secs(1);

/// Responder-side MAC checking and cookie issuing, shareable by all peers of a device.
///
/// The device counts handshake messages with a valid MAC1 per second; above the limit it is
/// under load and demands a valid MAC2, answering anything else with a cookie reply.
pub struct CookieChecker {
    secret: Zeroizing<[u8; 32]>,
    previous_secret: Option<Zeroizing<[u8; 32]>>,
    secret_born: Option<Duration>,
    limit_per_second: u32,
    window_start: Duration,
    count_in_window: u32,
}

impl fmt::Debug for CookieChecker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CookieChecker")
            .field("limit_per_second", &self.limit_per_second)
            .finish_non_exhaustive()
    }
}

/// A handshake message awaiting MAC checks.
pub(crate) struct Incoming<'a> {
    /// The caller's encoding of the sender's IP and port.
    pub(crate) source: &'a [u8],
    pub(crate) datagram: &'a [u8],
    pub(crate) mac1_offset: usize,
    pub(crate) sender: u32,
}

/// What to do with a handshake message after MAC checks.
pub(crate) enum Verdict {
    Process,
    SendCookie(CookieReply),
}

impl CookieChecker {
    /// A checker that considers itself under load above `handshakes_per_second`.
    #[must_use]
    pub fn new(handshakes_per_second: u32) -> Self {
        Self {
            secret: Zeroizing::new([0; 32]),
            previous_secret: None,
            secret_born: None,
            limit_per_second: handshakes_per_second,
            window_start: Duration::ZERO,
            count_in_window: 0,
        }
    }

    /// Checks MAC1 and, under load, MAC2 of a handshake message.
    pub(crate) fn verify(
        &mut self,
        now: Duration,
        random: &mut dyn RandomSource,
        identity: &StaticIdentity,
        incoming: &Incoming<'_>,
    ) -> Result<Verdict, WgError> {
        let Incoming {
            source,
            datagram: message,
            mac1_offset,
            sender,
        } = *incoming;
        let mac2_offset = mac1_offset.saturating_add(MAC_LEN);
        let (covered_by_mac1, rest) = message
            .split_at_checked(mac1_offset)
            .ok_or(WgError::MalformedMessage)?;
        let (received_mac1, received_mac2) = rest
            .split_at_checked(MAC_LEN)
            .ok_or(WgError::MalformedMessage)?;
        let expected_mac1 =
            mac(&identity.mac1_key, &[covered_by_mac1]).ok_or(WgError::InvalidMac)?;
        if !constant_time_eq(&expected_mac1, received_mac1) {
            return Err(WgError::InvalidMac);
        }

        if !self.count_and_check_load(now) {
            return Ok(Verdict::Process);
        }

        self.rotate_secret(now, random)?;
        let cookie = mac(self.secret.as_slice(), &[source]).ok_or(WgError::InvalidMac)?;
        let covered_by_mac2 = message
            .get(..mac2_offset)
            .ok_or(WgError::MalformedMessage)?;
        let mac2_matches = |cookie: &[u8; MAC_LEN]| {
            mac(cookie, &[covered_by_mac2])
                .is_some_and(|expected| constant_time_eq(&expected, received_mac2))
        };
        // A cookie issued just before rotation stays valid, as in wireguard-go.
        let previous_cookie = self
            .previous_secret
            .as_ref()
            .and_then(|secret| mac(secret.as_slice(), &[source]));
        if mac2_matches(&cookie) || previous_cookie.as_ref().is_some_and(mac2_matches) {
            return Ok(Verdict::Process);
        }

        let mut nonce = [0_u8; 24];
        random.fill_bytes(&mut nonce)?;
        let mut encrypted_cookie = [0_u8; 32];
        let (body, tag_slot) = encrypted_cookie.split_at_mut(MAC_LEN);
        body.copy_from_slice(&cookie);
        let tag = CookieAeadKey::from_bytes(&identity.cookie_key)
            .seal_in_place(&nonce, received_mac1, body)
            .map_err(|_| WgError::DecryptionFailed)?;
        tag_slot.copy_from_slice(&tag.to_bytes());
        Ok(Verdict::SendCookie(CookieReply {
            receiver: sender,
            nonce,
            encrypted_cookie,
        }))
    }

    fn count_and_check_load(&mut self, now: Duration) -> bool {
        if now.saturating_sub(self.window_start) >= LOAD_WINDOW {
            self.window_start = now;
            self.count_in_window = 0;
        }
        self.count_in_window = self.count_in_window.saturating_add(1);
        self.count_in_window > self.limit_per_second
    }

    fn rotate_secret(
        &mut self,
        now: Duration,
        random: &mut dyn RandomSource,
    ) -> Result<(), WgError> {
        let fresh = self
            .secret_born
            .is_some_and(|born| now.saturating_sub(born) < COOKIE_SECRET_LIFETIME);
        if !fresh {
            self.previous_secret = self
                .secret_born
                .filter(|born| now.saturating_sub(*born) < COOKIE_SECRET_LIFETIME.saturating_mul(2))
                .map(|_| self.secret.clone());
            random.fill_bytes(self.secret.as_mut_slice())?;
            self.secret_born = Some(now);
        }
        Ok(())
    }
}

/// Decrypts a cookie reply to a message whose MAC1 was `sent_mac1`.
pub(crate) fn open_cookie_reply(
    reply: &CookieReply,
    peer_cookie_key: &[u8; 32],
    sent_mac1: &[u8; MAC_LEN],
) -> Result<[u8; MAC_LEN], WgError> {
    let (body, tag) = reply.encrypted_cookie.split_at(MAC_LEN);
    let mut cookie = [0_u8; MAC_LEN];
    cookie.copy_from_slice(body);
    let tag: [u8; MAC_LEN] = tag.try_into().map_err(|_| WgError::MalformedMessage)?;
    CookieAeadKey::from_bytes(peer_cookie_key)
        .open_in_place(&reply.nonce, sent_mac1, &mut cookie, &Tag::from_bytes(tag))
        .map_err(|_| WgError::DecryptionFailed)?;
    Ok(cookie)
}

/// Writes MAC1, and MAC2 when we hold a cookie, into a serialized handshake message.
pub(crate) fn seal_macs(
    message: &mut [u8],
    mac1_offset: usize,
    mac1_key: &[u8; 32],
    cookie: Option<&[u8; MAC_LEN]>,
) -> Result<[u8; MAC_LEN], WgError> {
    let mac2_offset = mac1_offset.saturating_add(MAC_LEN);
    let mac1 = mac(
        mac1_key,
        &[message.get(..mac1_offset).ok_or(WgError::BufferTooSmall)?],
    )
    .ok_or(WgError::InvalidMac)?;
    message
        .get_mut(mac1_offset..mac2_offset)
        .ok_or(WgError::BufferTooSmall)?
        .copy_from_slice(&mac1);
    if let Some(cookie) = cookie {
        let mac2 = mac(
            cookie,
            &[message.get(..mac2_offset).ok_or(WgError::BufferTooSmall)?],
        )
        .ok_or(WgError::InvalidMac)?;
        message
            .get_mut(mac2_offset..mac2_offset.saturating_add(MAC_LEN))
            .ok_or(WgError::BufferTooSmall)?
            .copy_from_slice(&mac2);
    }
    Ok(mac1)
}
