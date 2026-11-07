//! WebAuthn passkey assertions: ES256 (ECDSA P-256 with SHA-256) over authenticator data.
//!
//! An assertion is valid for a challenge when, in order:
//!
//! 1. `client_data_json` is JSON whose `type` is `webauthn.get` and whose `challenge` is the
//!    challenge in unpadded base64url;
//! 2. `authenticator_data` starts with SHA-256 of the relying party id, and its flags have
//!    user presence and user verification set;
//! 3. the DER signature verifies under the passkey's key over
//!    `authenticator_data || SHA-256(client_data_json)`.

use alloc::string::String;

use data_encoding::BASE64URL_NOPAD;
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{DerSignature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

/// Length of an uncompressed SEC1 P-256 public key: `0x04 || x || y`.
pub const PASSKEY_KEY_LEN: usize = 65;

/// Authenticator data is at least the rp id hash, the flags byte and the sign counter.
const MIN_AUTHENTICATOR_DATA_LEN: usize = 32 + 1 + 4;

/// Flag bit: the user was present.
const USER_PRESENT: u8 = 0x01;

/// Flag bit: the user was verified, by biometrics or a PIN.
const USER_VERIFIED: u8 = 0x04;

/// Why an assertion was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PasskeyError {
    /// The public key is not an uncompressed SEC1 point on P-256.
    #[error("not an uncompressed P-256 public key")]
    BadKey,
    /// The client data is not JSON with string `type` and `challenge` fields.
    #[error("malformed client data")]
    BadClientData,
    /// The client data's `type` is not `webauthn.get`.
    #[error("client data is not for an assertion")]
    WrongType,
    /// The client data's `challenge` is not the expected one.
    #[error("wrong challenge")]
    WrongChallenge,
    /// The authenticator data is too short.
    #[error("malformed authenticator data")]
    BadAuthenticatorData,
    /// The authenticator data is for another relying party.
    #[error("wrong relying party")]
    WrongRelyingParty,
    /// The authenticator did not report a present, verified user.
    #[error("the user was not verified")]
    NotVerified,
    /// The signature is not DER, or does not verify.
    #[error("bad signature")]
    BadSignature,
}

/// A passkey's public key: a P-256 point.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PasskeyKey(VerifyingKey);

impl PasskeyKey {
    /// Parses an uncompressed SEC1 key, as [`PASSKEY_KEY_LEN`] bytes.
    pub fn from_sec1(bytes: &[u8]) -> Result<Self, PasskeyError> {
        if bytes.len() != PASSKEY_KEY_LEN || bytes.first() != Some(&0x04) {
            return Err(PasskeyError::BadKey);
        }
        VerifyingKey::from_sec1_bytes(bytes)
            .map(Self)
            .map_err(|_| PasskeyError::BadKey)
    }

    /// Checks a WebAuthn assertion by this key for `challenge` and relying party `rp_id`.
    pub fn verify_assertion(
        &self,
        rp_id: &str,
        challenge: &[u8],
        authenticator_data: &[u8],
        client_data_json: &[u8],
        signature: &[u8],
    ) -> Result<(), PasskeyError> {
        check_client_data(client_data_json, challenge)?;
        if authenticator_data.len() < MIN_AUTHENTICATOR_DATA_LEN {
            return Err(PasskeyError::BadAuthenticatorData);
        }
        if authenticator_data.get(..32) != Some(sha256(rp_id.as_bytes()).as_slice()) {
            return Err(PasskeyError::WrongRelyingParty);
        }
        let flags = authenticator_data.get(32).copied().unwrap_or(0);
        if flags & USER_PRESENT == 0 || flags & USER_VERIFIED == 0 {
            return Err(PasskeyError::NotVerified);
        }
        let signature =
            DerSignature::try_from(signature).map_err(|_| PasskeyError::BadSignature)?;
        let mut signed = alloc::vec::Vec::with_capacity(authenticator_data.len() + 32);
        signed.extend_from_slice(authenticator_data);
        signed.extend_from_slice(&sha256(client_data_json));
        self.0
            .verify(&signed, &signature)
            .map_err(|_| PasskeyError::BadSignature)
    }
}

impl core::fmt::Debug for PasskeyKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PasskeyKey")
    }
}

/// SHA-256, which WebAuthn uses for relying party ids and client data.
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// A challenge as WebAuthn carries it in client data: unpadded base64url.
#[must_use]
pub fn encode_challenge(challenge: &[u8]) -> String {
    BASE64URL_NOPAD.encode(challenge)
}

#[derive(Deserialize)]
struct ClientData<'a> {
    #[serde(rename = "type", borrow)]
    kind: &'a str,
    #[serde(borrow)]
    challenge: &'a str,
}

fn check_client_data(json: &[u8], challenge: &[u8]) -> Result<(), PasskeyError> {
    let data: ClientData<'_> =
        serde_json::from_slice(json).map_err(|_| PasskeyError::BadClientData)?;
    if data.kind != "webauthn.get" {
        return Err(PasskeyError::WrongType);
    }
    if data.challenge != encode_challenge(challenge) {
        return Err(PasskeyError::WrongChallenge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::format;
    use alloc::vec::Vec;

    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer as _;

    use super::{PasskeyError, PasskeyKey, encode_challenge, sha256};

    const RP: &str = "vpn.example.com";

    struct Signed {
        key: Vec<u8>,
        authenticator_data: Vec<u8>,
        client_data: Vec<u8>,
        signature: Vec<u8>,
    }

    fn sign(kind: &str, challenge: &[u8], rp: &str, flags: u8) -> Signed {
        let secret = SigningKey::from_slice(&[7; 32]).unwrap();
        let key = secret.verifying_key().to_sec1_bytes().to_vec();
        let mut authenticator_data = sha256(rp.as_bytes()).to_vec();
        authenticator_data.extend([flags, 0, 0, 0, 1]);
        let client_data = format!(
            r#"{{"type":"{kind}","challenge":"{}","origin":"https://{rp}","crossOrigin":false}}"#,
            encode_challenge(challenge)
        )
        .into_bytes();
        let mut message = authenticator_data.clone();
        message.extend(sha256(&client_data));
        let signature: p256::ecdsa::DerSignature = secret.sign(&message);
        Signed {
            key,
            authenticator_data,
            client_data,
            signature: signature.as_bytes().to_vec(),
        }
    }

    fn check(signed: &Signed, challenge: &[u8]) -> Result<(), PasskeyError> {
        PasskeyKey::from_sec1(&signed.key)?.verify_assertion(
            RP,
            challenge,
            &signed.authenticator_data,
            &signed.client_data,
            &signed.signature,
        )
    }

    #[test]
    fn accepts_a_verified_assertion_and_refuses_every_mismatch() {
        let good = sign("webauthn.get", b"op", RP, 0x05);
        assert_eq!(check(&good, b"op"), Ok(()));
        assert_eq!(check(&good, b"other"), Err(PasskeyError::WrongChallenge));
        let created = sign("webauthn.create", b"op", RP, 0x05);
        assert_eq!(check(&created, b"op"), Err(PasskeyError::WrongType));
        let elsewhere = sign("webauthn.get", b"op", "evil.example", 0x05);
        assert_eq!(
            check(&elsewhere, b"op"),
            Err(PasskeyError::WrongRelyingParty)
        );
        let unverified = sign("webauthn.get", b"op", RP, 0x01);
        assert_eq!(check(&unverified, b"op"), Err(PasskeyError::NotVerified));

        let mut tampered = sign("webauthn.get", b"op", RP, 0x05);
        if let Some(counter) = tampered.authenticator_data.last_mut() {
            *counter = 2;
        }
        assert_eq!(check(&tampered, b"op"), Err(PasskeyError::BadSignature));
        let mut garbled = sign("webauthn.get", b"op", RP, 0x05);
        garbled.signature.truncate(10);
        assert_eq!(check(&garbled, b"op"), Err(PasskeyError::BadSignature));
    }

    #[test]
    fn keys_must_be_uncompressed_points_on_the_curve() {
        let good = sign("webauthn.get", b"op", RP, 0x05);
        assert!(PasskeyKey::from_sec1(&good.key).is_ok());
        let mut off_curve = good.key.clone();
        if let Some(last) = off_curve.last_mut() {
            *last ^= 1;
        }
        assert_eq!(PasskeyKey::from_sec1(&off_curve), Err(PasskeyError::BadKey));
        assert_eq!(
            PasskeyKey::from_sec1(good.key.get(..33).unwrap()),
            Err(PasskeyError::BadKey)
        );
    }
}
