//! The message envelope shared by every kith protocol.
//!
//! ```text
//! magic "kt" (2 bytes) | message type (u8) | version (varint) | postcard body
//! ```

use alloc::vec::Vec;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{WireError, varint};

/// Every envelope starts with these bytes.
pub const MAGIC: [u8; 2] = *b"kt";

/// Default upper bound on an encoded message: 1 MiB.
pub const MAX_MESSAGE_LEN: usize = 1 << 20;

/// Identifies what an envelope carries. Each protocol reserves its own values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageType(pub u8);

/// A decoded envelope header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// What the body contains.
    pub message_type: MessageType,
    /// The body's schema version.
    pub version: u64,
}

/// Encodes `body` in an envelope.
pub fn encode<T: Serialize + ?Sized>(
    message_type: MessageType,
    version: u64,
    body: &T,
) -> Result<Vec<u8>, WireError> {
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&MAGIC);
    out.push(message_type.0);
    varint::encode(version, &mut out);
    let out = postcard::to_extend(body, out).map_err(|_| WireError::Encode)?;
    if out.len() > MAX_MESSAGE_LEN {
        return Err(too_large(out.len()));
    }
    Ok(out)
}

/// Splits an envelope into its header and body bytes.
pub fn decode_header(input: &[u8]) -> Result<(Header, &[u8]), WireError> {
    if input.len() > MAX_MESSAGE_LEN {
        return Err(too_large(input.len()));
    }
    let (magic, rest) = input
        .split_at_checked(MAGIC.len())
        .ok_or(WireError::Truncated)?;
    if magic != MAGIC {
        return Err(WireError::BadMagic);
    }
    let (&message_type, rest) = rest.split_first().ok_or(WireError::Truncated)?;
    let (version, used) = varint::decode(rest)?;
    let body = rest.get(used..).ok_or(WireError::Truncated)?;
    let header = Header {
        message_type: MessageType(message_type),
        version,
    };
    Ok((header, body))
}

/// Decodes an envelope of the expected type whose version is at most `max_version`.
///
/// Returns the version found so the caller can handle older schemas.
pub fn decode<T: DeserializeOwned>(
    input: &[u8],
    expected: MessageType,
    max_version: u64,
) -> Result<(u64, T), WireError> {
    let (header, body) = decode_header(input)?;
    if header.message_type != expected {
        return Err(WireError::UnexpectedType {
            expected: expected.0,
            found: header.message_type.0,
        });
    }
    if header.version > max_version {
        return Err(WireError::UnsupportedVersion {
            found: header.version,
            supported: max_version,
        });
    }
    Ok((header.version, decode_body(body)?))
}

/// Decodes a postcard body, rejecting trailing bytes.
pub fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, WireError> {
    let (value, rest) = postcard::take_from_bytes(body).map_err(|error| map_postcard(&error))?;
    if rest.is_empty() {
        Ok(value)
    } else {
        Err(WireError::TrailingBytes)
    }
}

fn map_postcard(error: &postcard::Error) -> WireError {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "every other postcard error means a malformed body"
    )]
    match error {
        postcard::Error::DeserializeUnexpectedEnd => WireError::Truncated,
        postcard::Error::DeserializeBadVarint => WireError::BadVarint,
        _ => WireError::Decode,
    }
}

fn too_large(len: usize) -> WireError {
    WireError::TooLarge {
        len: u64::try_from(len).unwrap_or(u64::MAX),
        max: u64::try_from(MAX_MESSAGE_LEN).unwrap_or(u64::MAX),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use proptest::prelude::*;
    use serde::{Deserialize, Serialize};

    use super::{MAGIC, MessageType, decode, decode_body, decode_header, encode};
    use crate::WireError;

    const HELLO: MessageType = MessageType(7);

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Hello {
        name: String,
        port: u16,
    }

    fn sample() -> Hello {
        Hello {
            name: "nas".into(),
            port: 41641,
        }
    }

    #[test]
    fn golden_bytes() {
        // Pins the format: changing these bytes is a protocol break.
        let bytes = encode(HELLO, 1, &sample()).unwrap();
        assert_eq!(
            bytes,
            [b'k', b't', 7, 1, 3, b'n', b'a', b's', 0xa9, 0xc5, 0x02]
        );
    }

    #[test]
    fn round_trips() {
        let bytes = encode(HELLO, 1, &sample()).unwrap();
        assert_eq!(decode::<Hello>(&bytes, HELLO, 1), Ok((1, sample())));
    }

    #[test]
    fn rejects_wrong_type_version_magic_and_trailing() {
        let bytes = encode(HELLO, 2, &sample()).unwrap();
        assert_eq!(
            decode::<Hello>(&bytes, MessageType(8), 2).err(),
            Some(WireError::UnexpectedType {
                expected: 8,
                found: 7
            })
        );
        assert_eq!(
            decode::<Hello>(&bytes, HELLO, 1).err(),
            Some(WireError::UnsupportedVersion {
                found: 2,
                supported: 1
            })
        );
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'x';
        assert_eq!(decode_header(&bad_magic).err(), Some(WireError::BadMagic));
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(
            decode::<Hello>(&trailing, HELLO, 2).err(),
            Some(WireError::TrailingBytes)
        );
        assert_eq!(decode_header(&MAGIC).err(), Some(WireError::Truncated));
    }

    #[test]
    fn rejects_oversized_input() {
        let huge = vec![0_u8; super::MAX_MESSAGE_LEN + 1];
        assert!(matches!(
            decode_header(&huge),
            Err(WireError::TooLarge { .. })
        ));
    }

    #[test]
    fn truncated_body_is_reported() {
        let bytes = encode(HELLO, 1, &sample()).unwrap();
        let cut = &bytes[..bytes.len() - 1];
        assert_eq!(
            decode::<Hello>(cut, HELLO, 1).err(),
            Some(WireError::Truncated)
        );
        assert_eq!(decode_body::<u8>(&[]), Err(WireError::Truncated));
    }

    proptest! {
        #[test]
        fn arbitrary_input_never_panics(input in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ignored = decode::<Hello>(&input, HELLO, u64::MAX);
        }

        #[test]
        fn arbitrary_bodies_round_trip(data in proptest::collection::vec(any::<u8>(), 0..256), version in any::<u64>()) {
            let bytes = encode(HELLO, version, &data).unwrap();
            prop_assert_eq!(decode::<Vec<u8>>(&bytes, HELLO, u64::MAX), Ok((version, data)));
        }
    }
}
