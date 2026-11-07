//! Passkeys: a small `WebAuthn` relying party for ES256 (P-256 with SHA-256), the one
//! algorithm every platform authenticator offers. Pure Rust, no OpenSSL.
//!
//! The page passes the browser's own JSON through: `PublicKeyCredential.toJSON()` for
//! answers, and `PublicKeyCredential.parseCreationOptionsFromJSON` /
//! `parseRequestOptionsFromJSON` for the options built here. Binary fields are base64url
//! without padding.
//!
//! Attestation is not asked for (`"none"`): a passkey proves who holds it, not which
//! device made it. Every ceremony requires user verification (Touch ID, Face ID, a PIN).

use core::fmt;

use data_encoding::BASE64URL_NOPAD;
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::store::StoredCredential;

/// COSE's number for ES256.
const ES256: i64 = -7;
/// How long the browser may take, in milliseconds.
pub(crate) const TIMEOUT_MS: u64 = 120_000;
/// The authenticator data flags.
const USER_PRESENT: u8 = 0x01;
const USER_VERIFIED: u8 = 0x04;
const ATTESTED: u8 = 0x40;
/// Longest credential id `WebAuthn` allows.
const MAX_CREDENTIAL_ID: usize = 1023;
/// How deep a CBOR value may nest.
const MAX_DEPTH: usize = 8;

/// Why a passkey answer was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WebAuthnError(pub(crate) &'static str);

impl fmt::Display for WebAuthnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "passkey refused: {}", self.0)
    }
}

impl core::error::Error for WebAuthnError {}

type Result<T> = core::result::Result<T, WebAuthnError>;

/// Base64url without padding; padded input is accepted too.
pub(crate) fn b64(bytes: &[u8]) -> String {
    BASE64URL_NOPAD.encode(bytes)
}

pub(crate) fn unb64(text: &str) -> Result<Vec<u8>> {
    BASE64URL_NOPAD
        .decode(text.trim_end_matches('=').as_bytes())
        .map_err(|_| WebAuthnError("not base64url"))
}

/// `RegistrationResponseJSON`: the answer to `navigator.credentials.create`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Registration {
    raw_id: String,
    #[serde(rename = "type")]
    kind: String,
    response: AttestationResponse,
}

#[derive(Debug, Clone, Deserialize)]
struct AttestationResponse {
    #[serde(rename = "clientDataJSON")]
    client_data_json: String,
    #[serde(rename = "attestationObject")]
    attestation_object: String,
}

/// `AuthenticationResponseJSON`: the answer to `navigator.credentials.get`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Authentication {
    raw_id: String,
    #[serde(rename = "type")]
    kind: String,
    response: AssertionResponse,
}

#[derive(Debug, Clone, Deserialize)]
struct AssertionResponse {
    #[serde(rename = "clientDataJSON")]
    client_data_json: String,
    #[serde(rename = "authenticatorData")]
    authenticator_data: String,
    signature: String,
}

/// An assertion, decoded: what the ledger checks for an account-signed op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Assertion {
    pub(crate) credential_id: Vec<u8>,
    pub(crate) authenticator_data: Vec<u8>,
    pub(crate) client_data_json: Vec<u8>,
    pub(crate) signature: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(default, rename = "crossOrigin")]
    cross_origin: bool,
}

/// Where answers must come from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Party<'a> {
    /// The relying party id: the Console's domain, or `localhost`.
    pub(crate) rp_id: &'a str,
    /// The page origins allowed in `clientDataJSON`.
    pub(crate) origins: &'a [String],
}

impl Registration {
    /// The challenge the browser signed, to find the ceremony it answers.
    pub(crate) fn challenge(&self) -> Result<String> {
        Ok(client_data(&unb64(&self.response.client_data_json)?)?.challenge)
    }
}

impl Authentication {
    /// The challenge the browser signed, to find the ceremony it answers.
    pub(crate) fn challenge(&self) -> Result<String> {
        Ok(client_data(&unb64(&self.response.client_data_json)?)?.challenge)
    }

    /// The decoded assertion.
    pub(crate) fn assertion(&self) -> Result<Assertion> {
        if self.kind != "public-key" {
            return Err(WebAuthnError("not a public-key credential"));
        }
        Ok(Assertion {
            credential_id: unb64(&self.raw_id)?,
            authenticator_data: unb64(&self.response.authenticator_data)?,
            client_data_json: unb64(&self.response.client_data_json)?,
            signature: unb64(&self.response.signature)?,
        })
    }
}

fn client_data(json: &[u8]) -> Result<ClientData> {
    serde_json::from_slice(json).map_err(|_| WebAuthnError("clientDataJSON does not parse"))
}

/// Checks `clientDataJSON` against the ceremony.
fn check_client(json: &[u8], kind: &str, challenge: &str, party: &Party<'_>) -> Result<()> {
    let data = client_data(json)?;
    if data.kind != kind {
        return Err(WebAuthnError("wrong ceremony type"));
    }
    if data.challenge != challenge {
        return Err(WebAuthnError("wrong challenge"));
    }
    if data.cross_origin || !party.origins.contains(&data.origin) {
        return Err(WebAuthnError("wrong origin"));
    }
    Ok(())
}

/// Checks the relying party hash and the flags at the start of authenticator data.
fn check_auth_data(auth_data: &[u8], rp_id: &str) -> Result<u8> {
    let rp_hash = auth_data
        .get(..32)
        .ok_or(WebAuthnError("authenticator data too short"))?;
    if rp_hash != Sha256::digest(rp_id.as_bytes()).as_slice() {
        return Err(WebAuthnError("made for another site"));
    }
    let flags = *auth_data
        .get(32)
        .ok_or(WebAuthnError("authenticator data too short"))?;
    if flags & USER_PRESENT == 0 || flags & USER_VERIFIED == 0 {
        return Err(WebAuthnError("the person was not verified"));
    }
    Ok(flags)
}

/// Checks a new passkey and returns it as the ledger keeps it.
pub(crate) fn verify_registration(
    registration: &Registration,
    challenge: &str,
    party: &Party<'_>,
) -> Result<StoredCredential> {
    if registration.kind != "public-key" {
        return Err(WebAuthnError("not a public-key credential"));
    }
    let client_json = unb64(&registration.response.client_data_json)?;
    check_client(&client_json, "webauthn.create", challenge, party)?;
    let object = unb64(&registration.response.attestation_object)?;
    let (attestation, _) = decode(&object, 0)?;
    let auth_data = attestation
        .get_text("authData")
        .and_then(Cbor::bytes)
        .ok_or(WebAuthnError("no authenticator data"))?;
    let flags = check_auth_data(auth_data, party.rp_id)?;
    if flags & ATTESTED == 0 {
        return Err(WebAuthnError("no credential in the answer"));
    }
    // rpIdHash (32), flags (1), signCount (4), aaguid (16), then the credential id.
    let id_len = auth_data
        .get(53..55)
        .and_then(|len| <[u8; 2]>::try_from(len).ok())
        .map(|len| usize::from(u16::from_be_bytes(len)))
        .ok_or(WebAuthnError("authenticator data too short"))?;
    if id_len == 0 || id_len > MAX_CREDENTIAL_ID {
        return Err(WebAuthnError("bad credential id"));
    }
    let id_end = 55usize.saturating_add(id_len);
    let credential_id = auth_data
        .get(55..id_end)
        .ok_or(WebAuthnError("authenticator data too short"))?
        .to_vec();
    if credential_id != unb64(&registration.raw_id)? {
        return Err(WebAuthnError("credential id mismatch"));
    }
    let key_bytes = auth_data
        .get(id_end..)
        .ok_or(WebAuthnError("no public key"))?;
    let (key, _) = decode(key_bytes, 0)?;
    let public_key = es256_key(&key)?;
    Ok(StoredCredential {
        credential_id,
        public_key,
        rp_id: party.rp_id.to_owned(),
    })
}

/// A COSE EC2 P-256 ES256 key as SEC1 uncompressed bytes, checked to be on the curve.
fn es256_key(key: &Cbor) -> Result<Vec<u8>> {
    let int = |label: i64| key.get_int(label).and_then(Cbor::int);
    if int(1) != Some(2) || int(3) != Some(ES256) || int(-1) != Some(1) {
        return Err(WebAuthnError("only ES256 passkeys are supported"));
    }
    let x = key.get_int(-2).and_then(Cbor::bytes);
    let y = key.get_int(-3).and_then(Cbor::bytes);
    let (Some(x), Some(y)) = (x, y) else {
        return Err(WebAuthnError("incomplete public key"));
    };
    if x.len() != 32 || y.len() != 32 {
        return Err(WebAuthnError("incomplete public key"));
    }
    let mut sec1 = Vec::with_capacity(65);
    sec1.push(4);
    sec1.extend_from_slice(x);
    sec1.extend_from_slice(y);
    VerifyingKey::from_sec1_bytes(&sec1).map_err(|_| WebAuthnError("not a P-256 point"))?;
    Ok(sec1)
}

/// Checks an assertion by the passkey with `public_key` (SEC1) made for `rp_id`. With
/// `party`, the origin is checked too; the ledger, which cannot know it, passes `None`.
pub(crate) fn verify_assertion(
    assertion: &Assertion,
    public_key: &[u8],
    rp_id: &str,
    challenge: &str,
    party: Option<&Party<'_>>,
) -> Result<()> {
    if let Some(party) = party {
        check_client(
            &assertion.client_data_json,
            "webauthn.get",
            challenge,
            party,
        )?;
    } else {
        let data = client_data(&assertion.client_data_json)?;
        if data.kind != "webauthn.get" || data.challenge != challenge {
            return Err(WebAuthnError("wrong challenge"));
        }
    }
    check_auth_data(&assertion.authenticator_data, rp_id)?;
    let key =
        VerifyingKey::from_sec1_bytes(public_key).map_err(|_| WebAuthnError("bad stored key"))?;
    let signature =
        Signature::from_der(&assertion.signature).map_err(|_| WebAuthnError("bad signature"))?;
    let mut signed = assertion.authenticator_data.clone();
    signed.extend_from_slice(&Sha256::digest(&assertion.client_data_json));
    key.verify(&signed, &signature)
        .map_err(|_| WebAuthnError("the signature does not match"))
}

/// `PublicKeyCredentialCreationOptionsJSON` for a new passkey.
pub(crate) fn creation_options(
    rp_id: &str,
    user_id: &[u8],
    name: &str,
    challenge: &str,
    exclude: &[Vec<u8>],
) -> Value {
    json!({
        "rp": {"id": rp_id, "name": "kith"},
        "user": {"id": b64(user_id), "name": name, "displayName": name},
        "challenge": challenge,
        "pubKeyCredParams": [{"type": "public-key", "alg": ES256}],
        "timeout": TIMEOUT_MS,
        "excludeCredentials": exclude
            .iter()
            .map(|id| json!({"type": "public-key", "id": b64(id)}))
            .collect::<Vec<_>>(),
        "authenticatorSelection": {
            "residentKey": "required",
            "requireResidentKey": true,
            "userVerification": "required",
        },
        "attestation": "none",
    })
}

/// `PublicKeyCredentialRequestOptionsJSON`; an empty `allow` lets the person pick any
/// passkey for this site.
pub(crate) fn request_options(rp_id: &str, challenge: &str, allow: &[Vec<u8>]) -> Value {
    json!({
        "rpId": rp_id,
        "challenge": challenge,
        "timeout": TIMEOUT_MS,
        "allowCredentials": allow
            .iter()
            .map(|id| json!({"type": "public-key", "id": b64(id)}))
            .collect::<Vec<_>>(),
        "userVerification": "required",
    })
}

/// The CBOR values passkeys use.
#[derive(Debug, Clone, PartialEq)]
enum Cbor {
    Uint(u64),
    /// `-1 - n`.
    Negative(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Self>),
    Map(Vec<(Self, Self)>),
    /// A simple value or a float, which nothing here reads.
    Other,
}

impl Cbor {
    fn int(&self) -> Option<i64> {
        match self {
            Self::Uint(n) => i64::try_from(*n).ok(),
            Self::Negative(n) => i64::try_from(*n).ok().map(|n| -1 - n),
            Self::Bytes(_) | Self::Text(_) | Self::Array(_) | Self::Map(_) | Self::Other => None,
        }
    }

    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            Self::Uint(_)
            | Self::Negative(_)
            | Self::Text(_)
            | Self::Array(_)
            | Self::Map(_)
            | Self::Other => None,
        }
    }

    fn entries(&self) -> &[(Self, Self)] {
        match self {
            Self::Map(entries) => entries,
            Self::Uint(_)
            | Self::Negative(_)
            | Self::Bytes(_)
            | Self::Text(_)
            | Self::Array(_)
            | Self::Other => &[],
        }
    }

    fn get_text(&self, key: &str) -> Option<&Self> {
        self.entries()
            .iter()
            .find(|(k, _)| matches!(k, Self::Text(text) if text == key))
            .map(|(_, value)| value)
    }

    fn get_int(&self, key: i64) -> Option<&Self> {
        self.entries()
            .iter()
            .find(|(k, _)| k.int() == Some(key))
            .map(|(_, value)| value)
    }
}

fn take(input: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    input
        .get(at..at.saturating_add(len))
        .filter(|slice| slice.len() == len)
        .ok_or(WebAuthnError("CBOR ends early"))
}

/// Decodes one definite-length CBOR value; returns it and the bytes it took.
fn decode(input: &[u8], depth: usize) -> Result<(Cbor, usize)> {
    if depth > MAX_DEPTH {
        return Err(WebAuthnError("CBOR nests too deep"));
    }
    let head = *input.first().ok_or(WebAuthnError("CBOR ends early"))?;
    let major = head >> 5;
    let info = head & 0x1f;
    let (arg, mut used) = match info {
        0..=23 => (u64::from(info), 1),
        24..=27 => {
            let len = 1usize << (info - 24);
            let bytes = take(input, 1, len)?;
            let arg = bytes
                .iter()
                .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
            (arg, 1 + len)
        }
        _ => return Err(WebAuthnError("indefinite CBOR is not supported")),
    };
    let remaining = input.len().saturating_sub(used);
    let count = usize::try_from(arg).map_err(|_| WebAuthnError("CBOR too long"))?;
    let value = match major {
        0 => Cbor::Uint(arg),
        1 => Cbor::Negative(arg),
        2 | 3 => {
            let bytes = take(input, used, count)?;
            used = used.saturating_add(count);
            if major == 2 {
                Cbor::Bytes(bytes.to_vec())
            } else {
                Cbor::Text(
                    core::str::from_utf8(bytes)
                        .map_err(|_| WebAuthnError("CBOR text is not UTF-8"))?
                        .to_owned(),
                )
            }
        }
        4 | 5 => {
            if count > remaining {
                return Err(WebAuthnError("CBOR ends early"));
            }
            let mut items = Vec::with_capacity(count);
            let mut pairs = Vec::with_capacity(if major == 5 { count } else { 0 });
            for _ in 0..count {
                let (key, len) = decode(input.get(used..).unwrap_or_default(), depth + 1)?;
                used = used.saturating_add(len);
                if major == 5 {
                    let (value, len) = decode(input.get(used..).unwrap_or_default(), depth + 1)?;
                    used = used.saturating_add(len);
                    pairs.push((key, value));
                } else {
                    items.push(key);
                }
            }
            if major == 5 {
                Cbor::Map(pairs)
            } else {
                Cbor::Array(items)
            }
        }
        6 => {
            let (inner, len) = decode(input.get(used..).unwrap_or_default(), depth + 1)?;
            used = used.saturating_add(len);
            inner
        }
        _ => Cbor::Other,
    };
    Ok((value, used))
}

/// A software passkey for tests: makes and signs answers as a browser would hand them on.
#[cfg(test)]
pub(crate) mod testing {
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer as _;
    use serde_json::{Value, json};
    use sha2::{Digest as _, Sha256};

    use super::b64;

    /// One passkey on one site.
    #[derive(Debug, Clone)]
    pub(crate) struct Authenticator {
        key: SigningKey,
        pub(crate) credential_id: Vec<u8>,
        rp_id: String,
        origin: String,
    }

    fn head(out: &mut Vec<u8>, major: u8, n: usize) {
        let major = major << 5;
        match u8::try_from(n) {
            Ok(small) if small < 24 => out.push(major | small),
            Ok(byte) => out.extend([major | 0x18, byte]),
            Err(_) => {
                out.push(major | 0x19);
                out.extend(u16::try_from(n).unwrap_or(u16::MAX).to_be_bytes());
            }
        }
    }

    fn bytes(out: &mut Vec<u8>, data: &[u8]) {
        head(out, 2, data.len());
        out.extend_from_slice(data);
    }

    fn text(out: &mut Vec<u8>, data: &str) {
        head(out, 3, data.len());
        out.extend_from_slice(data.as_bytes());
    }

    /// A small integer, positive or negative.
    fn int(out: &mut Vec<u8>, n: i8) {
        if n >= 0 {
            head(out, 0, usize::from(n.unsigned_abs()));
        } else {
            head(out, 1, usize::from((-1 - n).unsigned_abs()));
        }
    }

    impl Authenticator {
        /// A passkey from a fixed seed, so tests are repeatable.
        pub(crate) fn new(seed: u8, rp_id: &str, origin: &str) -> Option<Self> {
            let mut secret = [7u8; 32];
            secret[0] = seed.max(1);
            Some(Self {
                key: SigningKey::from_slice(&secret).ok()?,
                credential_id: vec![seed; 16],
                rp_id: rp_id.to_owned(),
                origin: origin.to_owned(),
            })
        }

        fn auth_data(&self, attested: bool) -> Vec<u8> {
            let mut data = Sha256::digest(self.rp_id.as_bytes()).to_vec();
            data.push(if attested { 0x45 } else { 0x05 });
            data.extend([0, 0, 0, 1]);
            if attested {
                data.extend([0u8; 16]);
                data.extend(
                    u16::try_from(self.credential_id.len())
                        .unwrap_or_default()
                        .to_be_bytes(),
                );
                data.extend_from_slice(&self.credential_id);
                let point = self.key.verifying_key().to_sec1_point(false);
                let sec1 = point.as_bytes();
                let mut cose = Vec::new();
                head(&mut cose, 5, 5);
                int(&mut cose, 1);
                int(&mut cose, 2);
                int(&mut cose, 3);
                int(&mut cose, -7);
                int(&mut cose, -1);
                int(&mut cose, 1);
                int(&mut cose, -2);
                bytes(&mut cose, sec1.get(1..33).unwrap_or_default());
                int(&mut cose, -3);
                bytes(&mut cose, sec1.get(33..65).unwrap_or_default());
                data.extend(cose);
            }
            data
        }

        fn client(&self, kind: &str, options: &Value) -> Vec<u8> {
            json!({
                "type": kind,
                "challenge": options["challenge"],
                "origin": self.origin,
                "crossOrigin": false,
            })
            .to_string()
            .into_bytes()
        }

        /// What `navigator.credentials.create(options)` would hand the page, as JSON.
        pub(crate) fn register(&self, options: &Value) -> Value {
            let mut object = Vec::new();
            head(&mut object, 5, 3);
            text(&mut object, "fmt");
            text(&mut object, "none");
            text(&mut object, "attStmt");
            head(&mut object, 5, 0);
            text(&mut object, "authData");
            bytes(&mut object, &self.auth_data(true));
            json!({
                "id": b64(&self.credential_id),
                "rawId": b64(&self.credential_id),
                "type": "public-key",
                "response": {
                    "clientDataJSON": b64(&self.client("webauthn.create", options)),
                    "attestationObject": b64(&object),
                    "transports": ["internal"],
                },
                "clientExtensionResults": {},
            })
        }

        /// What `navigator.credentials.get(options)` would hand the page, as JSON.
        pub(crate) fn assert(&self, options: &Value) -> Value {
            let auth_data = self.auth_data(false);
            let client = self.client("webauthn.get", options);
            let mut signed = auth_data.clone();
            signed.extend_from_slice(&Sha256::digest(&client));
            let signature: p256::ecdsa::Signature = self.key.sign(&signed);
            json!({
                "id": b64(&self.credential_id),
                "rawId": b64(&self.credential_id),
                "type": "public-key",
                "response": {
                    "clientDataJSON": b64(&client),
                    "authenticatorData": b64(&auth_data),
                    "signature": b64(signature.to_der().as_bytes()),
                    "userHandle": null,
                },
                "clientExtensionResults": {},
            })
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use super::testing::Authenticator;
    use super::*;

    type TestResult = core::result::Result<(), Box<dyn core::error::Error>>;

    #[test]
    fn a_registered_passkey_signs_assertions_for_its_site_only() -> TestResult {
        let origins = ["https://vpn.example.com".to_owned()];
        let party = Party {
            rp_id: "vpn.example.com",
            origins: &origins,
        };
        let passkey =
            Authenticator::new(3, "vpn.example.com", "https://vpn.example.com").ok_or("no key")?;
        let options = creation_options("vpn.example.com", b"user", "Ada", "Y2hhbGxlbmdl", &[]);
        let registration: Registration = serde_json::from_value(passkey.register(&options))?;
        assert_eq!(registration.challenge()?, "Y2hhbGxlbmdl");
        let credential = verify_registration(&registration, "Y2hhbGxlbmdl", &party)?;
        assert_eq!(credential.credential_id, passkey.credential_id);
        assert_eq!(
            verify_registration(&registration, "b3RoZXI", &party).err(),
            Some(WebAuthnError("wrong challenge"))
        );

        let request = request_options("vpn.example.com", "c2lnbg", &[]);
        let answer: Authentication = serde_json::from_value(passkey.assert(&request))?;
        let assertion = answer.assertion()?;
        let check = |rp: &str, challenge: &str| {
            verify_assertion(
                &assertion,
                &credential.public_key,
                rp,
                challenge,
                Some(&party),
            )
        };
        check("vpn.example.com", "c2lnbg")?;
        assert!(check("vpn.example.com", "b3RoZXI").is_err());
        assert!(check("evil.example.com", "c2lnbg").is_err());

        let elsewhere = ["https://evil.example.com".to_owned()];
        let wrong_origin = Party {
            rp_id: "vpn.example.com",
            origins: &elsewhere,
        };
        assert!(
            verify_assertion(
                &assertion,
                &credential.public_key,
                "vpn.example.com",
                "c2lnbg",
                Some(&wrong_origin)
            )
            .is_err()
        );

        let mut tampered = assertion.clone();
        if let Some(byte) = tampered.authenticator_data.last_mut() {
            *byte ^= 1;
        }
        assert!(
            verify_assertion(
                &tampered,
                &credential.public_key,
                "vpn.example.com",
                "c2lnbg",
                None
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn cbor_that_lies_about_its_length_is_refused() {
        assert!(decode(&[0x5a, 0xff, 0xff, 0xff, 0xff], 0).is_err());
        assert!(decode(&[0xbf], 0).is_err(), "indefinite map");
        assert!(decode(&[0x81; 64], 0).is_err(), "too deep");
    }
}
