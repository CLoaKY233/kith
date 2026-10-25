//! Invite tokens: the secret behind a `kith://join/…` link or QR code.
//!
//! ```text
//! kith://join/<base32(envelope || checksum)>
//! ```
//!
//! `envelope` is a [`message_types::INVITE`] envelope; `checksum` is the first
//! [`CHECKSUM_LEN`] bytes of its BLAKE3 hash under [`contexts::INVITE_CHECKSUM`]. Base32 is
//! RFC 4648 unpadded and parsed case-insensitively, so the uppercase QR form fits
//! alphanumeric mode.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use data_encoding::BASE32_NOPAD;
use kith_crypto::{Hash, SigningKey, VerifyingKey};
use kith_types::{NetworkId, NodeId, Timestamp};
use kith_wire::{WireError, envelope};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::bootstrap::{Bootstrap, BootstrapError};
use crate::{contexts, message_types};

/// The current (and only) invite version.
pub(crate) const INVITE_VERSION: u64 = 1;

/// The scheme and path every invite link starts with (matched case-insensitively).
pub const LINK_PREFIX: &str = "kith://join/";

/// The most bootstrap endpoints one invite may list.
pub const MAX_BOOTSTRAP: usize = 8;

/// The longest network name hint, in bytes.
pub const MAX_NETWORK_NAME_LEN: usize = 64;

/// The longest base32 payload accepted, in characters. Checked before decoding, which
/// also bounds every allocation made while parsing.
pub const MAX_LINK_PAYLOAD_LEN: usize = 1024;

/// Bytes of truncated checksum appended to the envelope.
const CHECKSUM_LEN: usize = 4;

/// Why an invite could not be built or parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum InviteError {
    /// The text does not start with [`LINK_PREFIX`].
    #[error("not a kith invite link")]
    WrongScheme,
    /// The payload exceeds [`MAX_LINK_PAYLOAD_LEN`] characters.
    #[error("invite payload of {len} characters exceeds {MAX_LINK_PAYLOAD_LEN}")]
    TooLarge {
        /// The payload length in characters.
        len: usize,
    },
    /// The payload is not valid unpadded base32.
    #[error("invite payload is not valid base32")]
    BadEncoding,
    /// The checksum does not match: the link was mistyped or truncated.
    #[error("invite checksum mismatch (mistyped link?)")]
    BadChecksum,
    /// Data follows the invite: a path, query, fragment or bytes after the envelope.
    #[error("unexpected data after the invite")]
    TrailingData,
    /// The envelope or its body is malformed.
    #[error("malformed invite: {0}")]
    Malformed(WireError),
    /// The invite has a version this build does not accept.
    #[error("unsupported invite version {0}")]
    UnsupportedVersion(u64),
    /// More than [`MAX_BOOTSTRAP`] bootstrap endpoints.
    #[error("{count} bootstrap endpoints exceed {MAX_BOOTSTRAP}")]
    TooManyBootstrap {
        /// How many were given.
        count: usize,
    },
    /// A bootstrap endpoint is invalid.
    #[error("invalid bootstrap endpoint: {0}")]
    Bootstrap(#[from] BootstrapError),
    /// The network name is longer than [`MAX_NETWORK_NAME_LEN`] bytes or contains control
    /// characters.
    #[error("network name must be at most {MAX_NETWORK_NAME_LEN} bytes without control characters")]
    InvalidNetworkName,
    /// The operating system's random source failed.
    #[error("random source unavailable")]
    Random,
}

impl From<WireError> for InviteError {
    fn from(error: WireError) -> Self {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "every other wire error, including future ones, means a malformed invite"
        )]
        match error {
            WireError::TrailingBytes => Self::TrailingData,
            other => Self::Malformed(other),
        }
    }
}

/// A validated invite to join a network.
///
/// A **bearer secret**: whoever holds it can join until it expires or is revoked. It
/// carries the seed of an Ed25519 invite key whose public half is in the ledger; the joiner
/// signs with it and never sends the secret. The token is unsigned, since the ledger, not
/// the token, decides membership; the checksum only catches typos. Not `Clone`; zeroizes
/// on drop.
pub struct InviteToken {
    network: NetworkId,
    sponsor: NodeId,
    invite_secret: Zeroizing<[u8; 32]>,
    bootstrap: Vec<Bootstrap>,
    expires: Timestamp,
    network_name: String,
}

/// Borrowed encoding of a token. Field order is the wire format.
#[derive(Serialize)]
struct InviteBodyRef<'token> {
    network: NetworkId,
    sponsor: NodeId,
    invite_secret: &'token [u8; 32],
    bootstrap: &'token [Bootstrap],
    expires: Timestamp,
    network_name: &'token str,
}

/// Owned decoding of a token, before validation. Zeroizes the secret on drop.
#[derive(Deserialize)]
struct InviteBody {
    network: NetworkId,
    sponsor: NodeId,
    invite_secret: [u8; 32],
    bootstrap: Vec<Bootstrap>,
    expires: Timestamp,
    network_name: String,
}

impl Drop for InviteBody {
    fn drop(&mut self) {
        self.invite_secret.zeroize();
    }
}

fn checksum(envelope_bytes: &[u8]) -> [u8; CHECKSUM_LEN] {
    let [first, second, third, fourth, ..] =
        Hash::of(contexts::INVITE_CHECKSUM, envelope_bytes).to_bytes();
    [first, second, third, fourth]
}

impl InviteToken {
    /// Builds a token from its parts, validating every field and the link length.
    pub fn from_parts(
        network: NetworkId,
        sponsor: NodeId,
        invite_secret: &[u8; 32],
        bootstrap: Vec<Bootstrap>,
        expires: Timestamp,
        network_name: String,
    ) -> Result<Self, InviteError> {
        if bootstrap.len() > MAX_BOOTSTRAP {
            return Err(InviteError::TooManyBootstrap {
                count: bootstrap.len(),
            });
        }
        for endpoint in &bootstrap {
            endpoint.validate()?;
        }
        if network_name.len() > MAX_NETWORK_NAME_LEN || network_name.chars().any(char::is_control) {
            return Err(InviteError::InvalidNetworkName);
        }
        let token = Self {
            network,
            sponsor,
            invite_secret: Zeroizing::new(*invite_secret),
            bootstrap,
            expires,
            network_name,
        };
        let len = token.payload()?.len();
        if len > MAX_LINK_PAYLOAD_LEN {
            return Err(InviteError::TooLarge { len });
        }
        Ok(token)
    }

    /// Builds a token with a fresh invite secret from the operating system's random source.
    #[cfg(feature = "getrandom")]
    pub fn generate(
        network: NetworkId,
        sponsor: NodeId,
        bootstrap: Vec<Bootstrap>,
        expires: Timestamp,
        network_name: String,
    ) -> Result<Self, InviteError> {
        let mut secret = Zeroizing::new([0_u8; 32]);
        kith_crypto::fill_random(secret.as_mut()).map_err(|_| InviteError::Random)?;
        Self::from_parts(network, sponsor, &secret, bootstrap, expires, network_name)
    }

    /// The network this invite joins.
    #[must_use]
    pub const fn network(&self) -> NetworkId {
        self.network
    }

    /// The device that created the invite; the joining device pins its control connection to it.
    #[must_use]
    pub const fn sponsor(&self) -> NodeId {
        self.sponsor
    }

    /// Where the joining device first knocks, in order of preference.
    #[must_use]
    pub fn bootstrap(&self) -> &[Bootstrap] {
        &self.bootstrap
    }

    /// When the invite stops being accepted.
    #[must_use]
    pub const fn expires(&self) -> Timestamp {
        self.expires
    }

    /// Whether the invite has expired at `now`. Sponsors make the authoritative check.
    #[must_use]
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now >= self.expires
    }

    /// The network's display name, a hint for the UI only: it is not authenticated.
    #[must_use]
    pub fn network_name(&self) -> &str {
        &self.network_name
    }

    /// The invite signing key, which the joining device uses to prove it holds the token.
    #[must_use]
    pub fn invite_key(&self) -> SigningKey {
        SigningKey::from_seed(&self.invite_secret)
    }

    /// The invite's public key, as recorded in the ledger.
    #[must_use]
    pub fn invite_public_key(&self) -> VerifyingKey {
        self.invite_key().verifying_key()
    }

    /// The link form, `kith://join/<lowercase base32>`. Contains the secret: zeroizes on drop.
    pub fn to_link(&self) -> Result<Zeroizing<String>, InviteError> {
        let mut payload = self.payload()?;
        payload.make_ascii_lowercase();
        Ok(join(LINK_PREFIX, &payload))
    }

    /// The uppercase form, `KITH://JOIN/<BASE32>`, which fits QR alphanumeric mode.
    /// Contains the secret: zeroizes on drop.
    pub fn to_qr_string(&self) -> Result<Zeroizing<String>, InviteError> {
        let payload = self.payload()?;
        let mut prefix = Zeroizing::new(String::from(LINK_PREFIX));
        prefix.make_ascii_uppercase();
        Ok(join(&prefix, &payload))
    }

    /// Parses a link in either case, strictly.
    pub fn parse(text: &str) -> Result<Self, InviteError> {
        let (scheme, payload) = text
            .split_at_checked(LINK_PREFIX.len())
            .ok_or(InviteError::WrongScheme)?;
        if !scheme.eq_ignore_ascii_case(LINK_PREFIX) {
            return Err(InviteError::WrongScheme);
        }
        if payload.len() > MAX_LINK_PAYLOAD_LEN {
            return Err(InviteError::TooLarge { len: payload.len() });
        }
        if payload.contains(['/', '?', '#']) {
            return Err(InviteError::TrailingData);
        }
        let mut upper = Zeroizing::new(String::from(payload));
        upper.make_ascii_uppercase();
        let bytes = Zeroizing::new(
            BASE32_NOPAD
                .decode(upper.as_bytes())
                .map_err(|_| InviteError::BadEncoding)?,
        );
        let split = bytes
            .len()
            .checked_sub(CHECKSUM_LEN)
            .ok_or(InviteError::Malformed(WireError::Truncated))?;
        let (envelope_bytes, found) = bytes
            .split_at_checked(split)
            .ok_or(InviteError::Malformed(WireError::Truncated))?;
        if found != checksum(envelope_bytes) {
            return Err(InviteError::BadChecksum);
        }
        let (version, body): (u64, InviteBody) =
            envelope::decode(envelope_bytes, message_types::INVITE, INVITE_VERSION)?;
        if version != INVITE_VERSION {
            return Err(InviteError::UnsupportedVersion(version));
        }
        let mut body = body;
        let bootstrap = core::mem::take(&mut body.bootstrap);
        let network_name = core::mem::take(&mut body.network_name);
        Self::from_parts(
            body.network,
            body.sponsor,
            &body.invite_secret,
            bootstrap,
            body.expires,
            network_name,
        )
    }

    /// The uppercase base32 payload. Contains the secret: zeroizes on drop.
    fn payload(&self) -> Result<Zeroizing<String>, InviteError> {
        let body = InviteBodyRef {
            network: self.network,
            sponsor: self.sponsor,
            invite_secret: &self.invite_secret,
            bootstrap: &self.bootstrap,
            expires: self.expires,
            network_name: &self.network_name,
        };
        let mut bytes = Zeroizing::new(envelope::encode(
            message_types::INVITE,
            INVITE_VERSION,
            &body,
        )?);
        let sum = checksum(&bytes);
        bytes.extend_from_slice(&sum);
        Ok(Zeroizing::new(BASE32_NOPAD.encode(&bytes)))
    }
}

fn join(prefix: &str, payload: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(
        prefix.len().saturating_add(payload.len()),
    ));
    out.push_str(prefix);
    out.push_str(payload);
    out
}

impl FromStr for InviteToken {
    type Err = InviteError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl fmt::Debug for InviteToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InviteToken")
            .field("network", &self.network)
            .field("sponsor", &self.sponsor)
            .field("invite_public_key", &self.invite_public_key())
            .field("invite_secret", &"<redacted>")
            .field("bootstrap", &self.bootstrap)
            .field("expires", &self.expires)
            .field("network_name", &self.network_name)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::net::{IpAddr, Ipv6Addr, SocketAddr};

    use proptest::prelude::*;

    use super::{
        BASE32_NOPAD, CHECKSUM_LEN, InviteError, InviteToken, LINK_PREFIX, MAX_BOOTSTRAP,
        MAX_LINK_PAYLOAD_LEN, checksum,
    };
    use crate::message_types;
    use crate::{Bootstrap, BootstrapAddr, BootstrapError, Hostname};
    use kith_types::{NetworkId, NodeId, Timestamp};
    use kith_wire::{WireError, envelope};

    const SECRET: [u8; 32] = [2; 32];
    const SPONSOR: NodeId = NodeId::from_bytes([3; 32]);
    const RELAY: NodeId = NodeId::from_bytes([4; 32]);

    fn sample() -> InviteToken {
        InviteToken::from_parts(
            NetworkId::from_bytes([1; 32]),
            SPONSOR,
            &SECRET,
            vec![
                Bootstrap::new(
                    SPONSOR,
                    BootstrapAddr::Socket("192.0.2.1:41641".parse().unwrap()),
                ),
                Bootstrap::new(
                    RELAY,
                    BootstrapAddr::Host(Hostname::new("relay.example").unwrap(), 443),
                ),
            ],
            Timestamp::from_millis(1_800_000_000_000),
            "home".into(),
        )
        .unwrap()
    }

    /// Wraps arbitrary envelope bytes as a link with a correct checksum.
    fn link_for(envelope_bytes: &[u8]) -> String {
        let mut bytes = envelope_bytes.to_vec();
        bytes.extend_from_slice(&checksum(envelope_bytes));
        format!(
            "{LINK_PREFIX}{}",
            BASE32_NOPAD.encode(&bytes).to_ascii_lowercase()
        )
    }

    fn assert_same(left: &InviteToken, right: &InviteToken) {
        assert_eq!(left.network(), right.network());
        assert_eq!(left.sponsor(), right.sponsor());
        assert_eq!(*left.invite_secret, *right.invite_secret);
        assert_eq!(left.bootstrap(), right.bootstrap());
        assert_eq!(left.expires(), right.expires());
        assert_eq!(left.network_name(), right.network_name());
    }

    #[test]
    fn golden_link() {
        // Pins the format: changing this text is a protocol break.
        assert_eq!(sample().to_link().unwrap().as_str(), GOLDEN);
    }

    const GOLDEN: &str = concat!(
        "kith://join/",
        "nn2ccaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibaebqgaydambqgaydambqgaydam",
        "bqgaydambqgaydambqgaydambqgaqcaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibq",
        "gaydambqgaydambqgaydambqgaydambqgaydambqgaydambqgagaaabadkofaicaibaeaqcaibaeaqcaib",
        "aeaqcaibaeaqcaibaeaqcaibaeaqcaiaqnojswyylzfzsxqylnobwgloydqcqpdqvrgqcgq33nmv5rk7rc",
    );

    #[test]
    fn round_trips_in_both_cases() {
        let token = sample();
        let link = token.to_link().unwrap();
        assert!(link.starts_with(LINK_PREFIX));
        assert_eq!(link.to_ascii_lowercase(), *link);
        assert_same(&InviteToken::parse(&link).unwrap(), &token);

        let qr = token.to_qr_string().unwrap();
        assert!(qr.starts_with("KITH://JOIN/"));
        assert_eq!(qr.to_ascii_uppercase(), *qr);
        assert_same(&qr.parse::<InviteToken>().unwrap(), &token);

        let mixed = format!("Kith://Join/{}", &link[LINK_PREFIX.len()..]);
        assert_same(&InviteToken::parse(&mixed).unwrap(), &token);
    }

    #[test]
    fn qr_string_uses_only_alphanumeric_mode_characters() {
        const QR_ALPHANUMERIC: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
        let qr = sample().to_qr_string().unwrap();
        assert!(qr.chars().all(|c| QR_ALPHANUMERIC.contains(c)));
    }

    #[test]
    fn invite_key_is_derived_from_the_secret() {
        let token = sample();
        let expected = kith_crypto::SigningKey::from_seed(&SECRET).verifying_key();
        assert_eq!(token.invite_public_key(), expected);
        assert_eq!(token.invite_key().verifying_key(), expected);
    }

    #[test]
    fn expiry_is_inclusive() {
        let token = sample();
        assert!(!token.is_expired(Timestamp::from_millis(1_799_999_999_999)));
        assert!(token.is_expired(token.expires()));
    }

    #[test]
    fn rejects_wrong_scheme() {
        let link = sample().to_link().unwrap();
        let payload = &link[LINK_PREFIX.len()..];
        for text in [
            format!("https://join/{payload}"),
            format!("kith://joins/{payload}"),
            String::from("kith://"),
            String::new(),
        ] {
            assert_eq!(
                InviteToken::parse(&text).err(),
                Some(InviteError::WrongScheme)
            );
        }
        // A multi-byte character straddling the prefix boundary must not panic.
        assert_eq!(
            InviteToken::parse("kith://joi\u{e9}x").err(),
            Some(InviteError::WrongScheme)
        );
    }

    #[test]
    fn rejects_oversized_payload() {
        let text = format!("{LINK_PREFIX}{}", "a".repeat(MAX_LINK_PAYLOAD_LEN + 1));
        assert_eq!(
            InviteToken::parse(&text).err(),
            Some(InviteError::TooLarge {
                len: MAX_LINK_PAYLOAD_LEN + 1
            })
        );
    }

    #[test]
    fn rejects_bad_base32() {
        let link = sample().to_link().unwrap();
        for bad in [
            format!("{}1", *link),
            format!("{}!", *link),
            format!("{} ", *link),
        ] {
            assert_eq!(
                InviteToken::parse(&bad).err(),
                Some(InviteError::BadEncoding)
            );
        }
    }

    #[test]
    fn rejects_bad_checksum() {
        let link = sample().to_link().unwrap();
        let mut typo: Vec<u8> = link.as_bytes().to_vec();
        let position = LINK_PREFIX.len() + 10;
        typo[position] = if typo[position] == b'a' { b'b' } else { b'a' };
        let typo = String::from_utf8(typo).unwrap();
        assert_eq!(
            InviteToken::parse(&typo).err(),
            Some(InviteError::BadChecksum)
        );
    }

    #[test]
    fn rejects_trailing_data() {
        let link = sample().to_link().unwrap();
        for suffix in ["/", "/extra", "?x=1", "#frag"] {
            let text = format!("{}{suffix}", *link);
            assert_eq!(
                InviteToken::parse(&text).err(),
                Some(InviteError::TrailingData)
            );
        }
        // Bytes after the envelope, behind a valid checksum.
        let mut payload = BASE32_NOPAD
            .decode(link[LINK_PREFIX.len()..].to_ascii_uppercase().as_bytes())
            .unwrap();
        payload.truncate(payload.len() - CHECKSUM_LEN);
        payload.push(0);
        assert_eq!(
            InviteToken::parse(&link_for(&payload)).err(),
            Some(InviteError::TrailingData)
        );
    }

    #[test]
    fn rejects_malformed_envelopes() {
        assert_eq!(
            InviteToken::parse(&format!("{LINK_PREFIX}aaaa")).err(),
            Some(InviteError::Malformed(WireError::Truncated))
        );
        let wrong_type = envelope::encode(message_types::SECRET_KEY, 1, &0_u8).unwrap();
        assert!(matches!(
            InviteToken::parse(&link_for(&wrong_type)),
            Err(InviteError::Malformed(WireError::UnexpectedType { .. }))
        ));
        let future = envelope::encode(message_types::INVITE, 2, &0_u8).unwrap();
        assert!(matches!(
            InviteToken::parse(&link_for(&future)),
            Err(InviteError::Malformed(WireError::UnsupportedVersion { .. }))
        ));
        let old = envelope::encode(message_types::INVITE, 0, &0_u8).unwrap();
        assert!(matches!(
            InviteToken::parse(&link_for(&old)),
            Err(InviteError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_invalid_fields() {
        let network = NetworkId::from_bytes([1; 32]);
        let expires = Timestamp::from_millis(1);
        let endpoint = Bootstrap::new(
            SPONSOR,
            BootstrapAddr::Socket("192.0.2.1:1".parse().unwrap()),
        );
        assert_eq!(
            InviteToken::from_parts(
                network,
                SPONSOR,
                &SECRET,
                vec![endpoint; MAX_BOOTSTRAP + 1],
                expires,
                String::new()
            )
            .err(),
            Some(InviteError::TooManyBootstrap {
                count: MAX_BOOTSTRAP + 1
            })
        );
        for name in ["x".repeat(65), String::from("bad\nname")] {
            assert_eq!(
                InviteToken::from_parts(network, SPONSOR, &SECRET, Vec::new(), expires, name).err(),
                Some(InviteError::InvalidNetworkName)
            );
        }
        let scoped = Bootstrap::new(
            SPONSOR,
            BootstrapAddr::Socket(SocketAddr::V6(core::net::SocketAddrV6::new(
                Ipv6Addr::LOCALHOST,
                1,
                0,
                7,
            ))),
        );
        assert_eq!(
            InviteToken::from_parts(
                network,
                SPONSOR,
                &SECRET,
                vec![scoped],
                expires,
                String::new()
            )
            .err(),
            Some(InviteError::Bootstrap(BootstrapError::ScopedAddress))
        );
        let long_host =
            Hostname::new(&format!("{0}.{0}.{0}.{1}", "a".repeat(63), "a".repeat(61))).unwrap();
        assert!(matches!(
            InviteToken::from_parts(
                network,
                SPONSOR,
                &SECRET,
                vec![Bootstrap::new(SPONSOR, BootstrapAddr::Host(long_host, 1)); MAX_BOOTSTRAP],
                expires,
                String::new()
            ),
            Err(InviteError::TooLarge { .. })
        ));
    }

    #[test]
    fn rejects_non_canonical_hostname_on_the_wire() {
        // Hand-build a body whose hostname is uppercase; only lowercase is canonical.
        #[derive(serde::Serialize)]
        enum Wire {
            #[expect(dead_code, reason = "keeps variant indexes aligned with the real enum")]
            V4([u8; 4], u16),
            #[expect(dead_code, reason = "keeps variant indexes aligned with the real enum")]
            V6([u8; 16], u16),
            Host(String, u16),
        }
        let body = (
            [1_u8; 32],
            [3_u8; 32],
            SECRET,
            vec![([3_u8; 32], Wire::Host("RELAY.example".into(), 1))],
            1_u64,
            "",
        );
        let bytes = envelope::encode(message_types::INVITE, 1, &body).unwrap();
        assert_eq!(
            InviteToken::parse(&link_for(&bytes)).err(),
            Some(InviteError::Malformed(WireError::Decode))
        );
    }

    #[test]
    fn debug_hides_the_secret() {
        let token = sample();
        let text = format!("{token:?}");
        let link = token.to_link().unwrap();
        assert!(text.contains("<redacted>"));
        assert!(text.contains("home"));
        assert!(!text.contains("2, 2"));
        assert!(!text.contains("0202"));
        assert!(!text.contains(&link[LINK_PREFIX.len()..]));
    }

    #[cfg(feature = "getrandom")]
    #[test]
    fn generated_tokens_differ() {
        let make = || {
            InviteToken::generate(
                NetworkId::from_bytes([1; 32]),
                SPONSOR,
                Vec::new(),
                Timestamp::MAX,
                String::from("net"),
            )
            .unwrap()
        };
        assert_ne!(make().invite_public_key(), make().invite_public_key());
    }

    fn bootstrap_strategy() -> impl Strategy<Value = Bootstrap> {
        (any::<[u8; 32]>(), addr_strategy())
            .prop_map(|(node, addr)| Bootstrap::new(NodeId::from_bytes(node), addr))
    }

    fn addr_strategy() -> impl Strategy<Value = BootstrapAddr> {
        prop_oneof![
            (any::<[u8; 4]>(), any::<u16>()).prop_map(|(ip, port)| BootstrapAddr::Socket(
                SocketAddr::new(IpAddr::from(ip), port)
            )),
            (any::<[u8; 16]>(), any::<u16>()).prop_map(|(ip, port)| BootstrapAddr::Socket(
                SocketAddr::new(IpAddr::from(ip), port)
            )),
            (
                "[a-z0-9]([a-z0-9-]{0,4}[a-z0-9])?(\\.[a-z]{1,6}){0,2}",
                any::<u16>()
            )
                .prop_filter_map("valid hostname", |(name, port)| {
                    Hostname::new(&name)
                        .ok()
                        .map(|host| BootstrapAddr::Host(host, port))
                }),
        ]
    }

    proptest! {
        #[test]
        fn arbitrary_text_never_panics(text in ".{0,200}") {
            let _ignored = InviteToken::parse(&text);
            let _ignored = InviteToken::parse(&format!("{LINK_PREFIX}{text}"));
        }

        #[test]
        fn arbitrary_payload_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
            let _ignored = InviteToken::parse(&link_for(&bytes));
            let text = format!("{LINK_PREFIX}{}", BASE32_NOPAD.encode(&bytes));
            let _ignored = InviteToken::parse(&text);
        }

        #[test]
        fn valid_tokens_round_trip(
            network in any::<[u8; 32]>(),
            sponsor in any::<[u8; 32]>(),
            secret in any::<[u8; 32]>(),
            bootstrap in proptest::collection::vec(bootstrap_strategy(), 0..=MAX_BOOTSTRAP),
            expires in any::<u64>(),
            name in "[ -~]{0,64}",
        ) {
            let token = InviteToken::from_parts(
                NetworkId::from_bytes(network),
                NodeId::from_bytes(sponsor),
                &secret,
                bootstrap,
                Timestamp::from_millis(expires),
                name,
            ).unwrap();
            assert_same(&InviteToken::parse(&token.to_link().unwrap()).unwrap(), &token);
            assert_same(&InviteToken::parse(&token.to_qr_string().unwrap()).unwrap(), &token);
        }
    }
}
