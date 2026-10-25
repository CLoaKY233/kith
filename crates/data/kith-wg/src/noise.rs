//! The `Noise_IKpsk2` handshake (whitepaper §5.4.2–§5.4.5).

use core::fmt;

use kith_crypto::wireguard::{SecretBytes, hash, kdf1, kdf2, kdf3};
use kith_crypto::{AeadKey, AgreementPublic, AgreementSecret, Nonce, TAG_LEN, Tag};
use zeroize::Zeroizing;

use crate::WgError;
use crate::messages::{Initiation, MAC_LEN, Response};
use crate::random::RandomSource;
use crate::time::Tai64N;

const CONSTRUCTION: &[u8] = b"Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";
const IDENTIFIER: &[u8] = b"WireGuard v1 zx2c4 Jason@zx2c4.com";
pub(crate) const LABEL_MAC1: &[u8] = b"mac1----";
pub(crate) const LABEL_COOKIE: &[u8] = b"cookie--";

/// Our long-term X25519 identity, shared by every peer of a device.
pub struct StaticIdentity {
    secret: AgreementSecret,
    public: [u8; 32],
    /// MAC1 key that peers use towards us.
    pub(crate) mac1_key: [u8; 32],
    /// Key under which we seal cookie replies.
    pub(crate) cookie_key: [u8; 32],
}

impl StaticIdentity {
    /// Builds an identity from a 32-byte X25519 private key.
    #[must_use]
    pub fn from_secret_bytes(secret: [u8; 32]) -> Self {
        let secret = AgreementSecret::from_bytes(secret);
        let public = secret.public_key().to_bytes();
        Self {
            secret,
            mac1_key: hash(&[LABEL_MAC1, &public]),
            cookie_key: hash(&[LABEL_COOKIE, &public]),
            public,
        }
    }

    /// Our public key.
    #[must_use]
    pub const fn public_key(&self) -> [u8; 32] {
        self.public
    }

    fn dh(&self, public: &[u8; 32]) -> Result<SecretBytes, WgError> {
        dh(&self.secret, public)
    }
}

impl fmt::Debug for StaticIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticIdentity")
            .field("public", &AgreementPublic::from_bytes(self.public))
            .finish_non_exhaustive()
    }
}

fn dh(secret: &AgreementSecret, public: &[u8; 32]) -> Result<SecretBytes, WgError> {
    secret
        .agree(&AgreementPublic::from_bytes(*public))
        .map(|shared| Zeroizing::new(*shared.as_bytes()))
        .map_err(|_| WgError::LowOrderKey)
}

/// Precomputed per-peer handshake material.
pub(crate) struct PeerKeys {
    pub(crate) public: [u8; 32],
    preshared: SecretBytes,
    static_static: SecretBytes,
    /// MAC1 key for messages we send to this peer.
    pub(crate) mac1_key: [u8; 32],
    /// Key under which this peer seals cookie replies to us.
    pub(crate) cookie_key: [u8; 32],
}

impl PeerKeys {
    pub(crate) fn new(
        identity: &StaticIdentity,
        public: [u8; 32],
        preshared: Option<&[u8; 32]>,
    ) -> Result<Self, WgError> {
        Ok(Self {
            public,
            preshared: Zeroizing::new(preshared.copied().unwrap_or([0; 32])),
            static_static: identity.dh(&public)?,
            mac1_key: hash(&[LABEL_MAC1, &public]),
            cookie_key: hash(&[LABEL_COOKIE, &public]),
        })
    }
}

/// The two transport keys of a completed handshake.
pub(crate) struct TransportKeys {
    pub(crate) send: AeadKey,
    pub(crate) receive: AeadKey,
}

/// Running Noise state: chaining key and transcript hash.
struct SymmetricState {
    chaining: SecretBytes,
    hash: [u8; 32],
}

impl SymmetricState {
    fn initial(responder_public: &[u8; 32]) -> Self {
        let chaining = hash(&[CONSTRUCTION]);
        let transcript = hash(&[&hash(&[&chaining, IDENTIFIER]), responder_public]);
        Self {
            chaining: Zeroizing::new(chaining),
            hash: transcript,
        }
    }

    fn mix_hash(&mut self, data: &[u8]) {
        self.hash = hash(&[&self.hash, data]);
    }

    fn mix_key(&mut self, input: &[u8]) {
        self.chaining = kdf1(&self.chaining, input);
    }

    fn mix_key_with_output(&mut self, input: &[u8]) -> SecretBytes {
        let (chaining, key) = kdf2(&self.chaining, input);
        self.chaining = chaining;
        key
    }

    fn seal<const N: usize, const M: usize>(
        &mut self,
        key: &SecretBytes,
        plaintext: &[u8; N],
    ) -> Result<[u8; M], WgError> {
        let mut out = [0_u8; M];
        let (body, tag_slot) = out.split_at_mut(N);
        body.copy_from_slice(plaintext);
        let tag = AeadKey::from_bytes(key)
            .seal_in_place(Nonce::from_counter(0), &self.hash, body)
            .map_err(|_| WgError::DecryptionFailed)?;
        tag_slot.copy_from_slice(&tag.to_bytes());
        self.mix_hash(&out);
        Ok(out)
    }

    fn open<const N: usize, const M: usize>(
        &mut self,
        key: &SecretBytes,
        sealed: &[u8; M],
    ) -> Result<Zeroizing<[u8; N]>, WgError> {
        let (body, tag) = sealed
            .split_at_checked(N)
            .ok_or(WgError::MalformedMessage)?;
        let tag: [u8; TAG_LEN] = tag.try_into().map_err(|_| WgError::MalformedMessage)?;
        let mut plaintext = Zeroizing::new([0_u8; N]);
        plaintext.copy_from_slice(body);
        AeadKey::from_bytes(key)
            .open_in_place(
                Nonce::from_counter(0),
                &self.hash,
                plaintext.as_mut_slice(),
                &Tag::from_bytes(tag),
            )
            .map_err(|_| WgError::DecryptionFailed)?;
        self.mix_hash(sealed);
        Ok(plaintext)
    }

    fn split(&self, initiator: bool) -> TransportKeys {
        let (first, second) = kdf2(&self.chaining, &[]);
        let (send, receive) = if initiator {
            (first, second)
        } else {
            (second, first)
        };
        TransportKeys {
            send: AeadKey::from_bytes(&send),
            receive: AeadKey::from_bytes(&receive),
        }
    }
}

fn generate_ephemeral(random: &mut dyn RandomSource) -> Result<AgreementSecret, WgError> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    random.fill_bytes(bytes.as_mut_slice())?;
    Ok(AgreementSecret::from_bytes(*bytes))
}

/// An initiation we sent and are awaiting the response to.
pub(crate) struct InitiatorHandshake {
    pub(crate) local_index: u32,
    ephemeral: AgreementSecret,
    state: SymmetricState,
}

/// Builds an initiation without MACs (§5.4.2).
pub(crate) fn create_initiation(
    identity: &StaticIdentity,
    peer: &PeerKeys,
    local_index: u32,
    timestamp: Tai64N,
    random: &mut dyn RandomSource,
) -> Result<(Initiation, InitiatorHandshake), WgError> {
    let mut state = SymmetricState::initial(&peer.public);
    let ephemeral = generate_ephemeral(random)?;
    let ephemeral_public = ephemeral.public_key().to_bytes();
    state.mix_key(&ephemeral_public);
    state.mix_hash(&ephemeral_public);
    let key = state.mix_key_with_output(dh(&ephemeral, &peer.public)?.as_slice());
    let encrypted_static = state.seal(&key, &identity.public)?;
    let key = state.mix_key_with_output(peer.static_static.as_slice());
    let encrypted_timestamp = state.seal(&key, &timestamp.to_bytes())?;
    Ok((
        Initiation {
            sender: local_index,
            ephemeral: ephemeral_public,
            encrypted_static,
            encrypted_timestamp,
            mac1: [0; MAC_LEN],
            mac2: [0; MAC_LEN],
        },
        InitiatorHandshake {
            local_index,
            ephemeral,
            state,
        },
    ))
}

/// An initiation whose static key we decrypted but have not yet authenticated.
pub(crate) struct HalfInitiation {
    pub(crate) peer_public: [u8; 32],
    remote_index: u32,
    remote_ephemeral: [u8; 32],
    encrypted_timestamp: [u8; 28],
    state: SymmetricState,
}

/// First half of consuming an initiation: needs only our identity, and reveals who sent it.
pub(crate) fn begin_initiation(
    identity: &StaticIdentity,
    message: &Initiation,
) -> Result<HalfInitiation, WgError> {
    let mut state = SymmetricState::initial(&identity.public);
    state.mix_key(&message.ephemeral);
    state.mix_hash(&message.ephemeral);
    let key = state.mix_key_with_output(identity.dh(&message.ephemeral)?.as_slice());
    let peer_public = state.open::<32, 48>(&key, &message.encrypted_static)?;
    Ok(HalfInitiation {
        peer_public: *peer_public,
        remote_index: message.sender,
        remote_ephemeral: message.ephemeral,
        encrypted_timestamp: message.encrypted_timestamp,
        state,
    })
}

/// Second half: authenticates the sender's static key and returns its timestamp.
pub(crate) fn finish_initiation(
    half: HalfInitiation,
    peer: &PeerKeys,
) -> Result<(Tai64N, ResponderHandshake), WgError> {
    let HalfInitiation {
        remote_index,
        remote_ephemeral,
        encrypted_timestamp,
        mut state,
        ..
    } = half;
    let key = state.mix_key_with_output(peer.static_static.as_slice());
    let timestamp = state.open::<12, 28>(&key, &encrypted_timestamp)?;
    Ok((
        Tai64N::from_bytes(*timestamp),
        ResponderHandshake {
            remote_index,
            remote_ephemeral,
            state,
        },
    ))
}

/// A validated initiation, ready to answer.
pub(crate) struct ResponderHandshake {
    pub(crate) remote_index: u32,
    remote_ephemeral: [u8; 32],
    state: SymmetricState,
}

/// Builds a response without MACs and derives the session (§5.4.3, §5.4.5).
pub(crate) fn create_response(
    handshake: ResponderHandshake,
    peer: &PeerKeys,
    local_index: u32,
    random: &mut dyn RandomSource,
) -> Result<(Response, TransportKeys), WgError> {
    let ResponderHandshake {
        remote_index,
        remote_ephemeral,
        mut state,
    } = handshake;
    let ephemeral = generate_ephemeral(random)?;
    let ephemeral_public = ephemeral.public_key().to_bytes();
    state.mix_key(&ephemeral_public);
    state.mix_hash(&ephemeral_public);
    state.mix_key(dh(&ephemeral, &remote_ephemeral)?.as_slice());
    state.mix_key(dh(&ephemeral, &peer.public)?.as_slice());
    let key = mix_preshared(&mut state, peer);
    let encrypted_nothing = state.seal::<0, 16>(&key, &[])?;
    Ok((
        Response {
            sender: local_index,
            receiver: remote_index,
            ephemeral: ephemeral_public,
            encrypted_nothing,
            mac1: [0; MAC_LEN],
            mac2: [0; MAC_LEN],
        },
        state.split(false),
    ))
}

/// Consumes a response to our initiation and derives the session.
pub(crate) fn consume_response(
    handshake: &InitiatorHandshake,
    identity: &StaticIdentity,
    peer: &PeerKeys,
    message: &Response,
) -> Result<TransportKeys, WgError> {
    let mut state = SymmetricState {
        chaining: handshake.state.chaining.clone(),
        hash: handshake.state.hash,
    };
    state.mix_key(&message.ephemeral);
    state.mix_hash(&message.ephemeral);
    state.mix_key(dh(&handshake.ephemeral, &message.ephemeral)?.as_slice());
    state.mix_key(identity.dh(&message.ephemeral)?.as_slice());
    let key = mix_preshared(&mut state, peer);
    state.open::<0, 16>(&key, &message.encrypted_nothing)?;
    Ok(state.split(true))
}

fn mix_preshared(state: &mut SymmetricState, peer: &PeerKeys) -> SecretBytes {
    let (chaining, tau, key) = kdf3(&state.chaining, peer.preshared.as_slice());
    state.chaining = chaining;
    state.mix_hash(tau.as_slice());
    key
}
