//! Request codes: a `kith://request/…` link that tells a new device where to ask to join.
//!
//! ```text
//! kith://request/<base32(envelope || checksum)>
//! ```
//!
//! Same layout as an invite link, but it holds no secret: anyone may share it, and a device
//! that uses it only gets in once an admin approves.

use alloc::string::String;
use alloc::vec::Vec;

use data_encoding::BASE32_NOPAD;
use kith_crypto::Hash;
use kith_types::NetworkId;
use kith_wire::{WireError, envelope};
use serde::{Deserialize, Serialize};

use crate::bootstrap::Bootstrap;
use crate::{InviteError, MAX_BOOTSTRAP, MAX_LINK_PAYLOAD_LEN, MAX_NETWORK_NAME_LEN};
use crate::{contexts, message_types};

const REQUEST_CODE_VERSION: u64 = 1;

/// The scheme and path every request code starts with (matched case-insensitively).
pub const REQUEST_PREFIX: &str = "kith://request/";

const CHECKSUM_LEN: usize = 4;

/// A validated request code: the network and members to ask. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCode {
    network: NetworkId,
    network_name: String,
    bootstrap: Vec<Bootstrap>,
}

impl RequestCode {
    /// Builds a code, validating the name, the bootstrap entries and the link length.
    pub fn new(
        network: NetworkId,
        network_name: String,
        bootstrap: Vec<Bootstrap>,
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
        let code = Self {
            network,
            network_name,
            bootstrap,
        };
        let len = code.payload()?.len();
        if len > MAX_LINK_PAYLOAD_LEN {
            return Err(InviteError::TooLarge { len });
        }
        Ok(code)
    }

    /// The network to ask to join.
    #[must_use]
    pub const fn network(&self) -> NetworkId {
        self.network
    }

    /// The network's display name; a hint, not authenticated.
    #[must_use]
    pub fn network_name(&self) -> &str {
        &self.network_name
    }

    /// Members to ask, best first.
    #[must_use]
    pub fn bootstrap(&self) -> &[Bootstrap] {
        &self.bootstrap
    }

    /// The link form, `kith://request/<lowercase base32>`.
    pub fn to_link(&self) -> Result<String, InviteError> {
        let mut link = String::from(REQUEST_PREFIX);
        link.push_str(&self.payload()?.to_ascii_lowercase());
        Ok(link)
    }

    /// Parses a link in either case, strictly.
    pub fn parse(text: &str) -> Result<Self, InviteError> {
        let (scheme, payload) = text
            .split_at_checked(REQUEST_PREFIX.len())
            .ok_or(InviteError::WrongScheme)?;
        if !scheme.eq_ignore_ascii_case(REQUEST_PREFIX) {
            return Err(InviteError::WrongScheme);
        }
        if payload.len() > MAX_LINK_PAYLOAD_LEN {
            return Err(InviteError::TooLarge { len: payload.len() });
        }
        if payload.contains(['/', '?', '#']) {
            return Err(InviteError::TrailingData);
        }
        let bytes = BASE32_NOPAD
            .decode(payload.to_ascii_uppercase().as_bytes())
            .map_err(|_| InviteError::BadEncoding)?;
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
        let (version, code): (u64, Self) = envelope::decode(
            envelope_bytes,
            message_types::REQUEST_CODE,
            REQUEST_CODE_VERSION,
        )?;
        if version != REQUEST_CODE_VERSION {
            return Err(InviteError::UnsupportedVersion(version));
        }
        Self::new(code.network, code.network_name, code.bootstrap)
    }

    fn payload(&self) -> Result<String, InviteError> {
        let mut bytes = envelope::encode(message_types::REQUEST_CODE, REQUEST_CODE_VERSION, self)?;
        let sum = checksum(&bytes);
        bytes.extend_from_slice(&sum);
        Ok(BASE32_NOPAD.encode(&bytes))
    }
}

fn checksum(envelope_bytes: &[u8]) -> [u8; CHECKSUM_LEN] {
    let [first, second, third, fourth, ..] =
        Hash::of(contexts::REQUEST_CODE_CHECKSUM, envelope_bytes).to_bytes();
    [first, second, third, fourth]
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;
    use core::net::SocketAddr;

    use kith_types::{NetworkId, NodeId};

    use super::RequestCode;
    use crate::{Bootstrap, BootstrapAddr, InviteError};

    #[test]
    fn a_request_code_round_trips_and_rejects_typos_and_invites() {
        let addr: SocketAddr = "192.0.2.7:41641".parse().unwrap();
        let code = RequestCode::new(
            NetworkId::from_bytes([1; 32]),
            String::from("home"),
            vec![Bootstrap::new(
                NodeId::from_bytes([2; 32]),
                BootstrapAddr::Socket(addr),
            )],
        )
        .unwrap();
        let link = code.to_link().unwrap();
        assert!(link.starts_with("kith://request/"));
        assert_eq!(
            RequestCode::parse(&link.to_ascii_uppercase()).unwrap(),
            code
        );

        let mut typo = link.clone().into_bytes();
        let last = typo.last_mut().unwrap();
        *last = if *last == b'a' { b'b' } else { b'a' };
        assert!(RequestCode::parse(core::str::from_utf8(&typo).unwrap()).is_err());
        assert_eq!(
            RequestCode::parse(&link.replace("request", "join")),
            Err(InviteError::WrongScheme)
        );
    }
}
