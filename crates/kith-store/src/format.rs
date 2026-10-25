//! Value file encoding. The layout is documented at the crate root.

use crate::crc::Crc32;

/// Largest value a store accepts, in bytes (16 MiB).
pub const MAX_VALUE_LEN: usize = 16 * 1024 * 1024;

pub(crate) const MAGIC: [u8; 4] = *b"KITH";
pub(crate) const VERSION: u8 = 1;
pub(crate) const HEADER_LEN: usize = 16;

fn checksum(prefix: &[u8], payload: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(prefix);
    crc.update(payload);
    crc.finish()
}

/// Encodes `payload`; the caller has checked it against [`MAX_VALUE_LEN`].
pub(crate) fn encode(payload: &[u8], length: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN.saturating_add(payload.len()));
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&[0; 3]);
    out.extend_from_slice(&length.to_le_bytes());
    let crc = checksum(&out, payload);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Decodes a value file, returning the payload or what check failed.
pub(crate) fn decode(bytes: &[u8]) -> Result<&[u8], &'static str> {
    let (header, payload) = bytes
        .split_first_chunk::<HEADER_LEN>()
        .ok_or("truncated header")?;
    let [
        m0,
        m1,
        m2,
        m3,
        version,
        r0,
        r1,
        r2,
        l0,
        l1,
        l2,
        l3,
        c0,
        c1,
        c2,
        c3,
    ] = *header;
    if [m0, m1, m2, m3] != MAGIC {
        return Err("bad magic");
    }
    if version != VERSION {
        return Err("unsupported version");
    }
    if [r0, r1, r2] != [0; 3] {
        return Err("reserved bytes not zero");
    }
    let length = usize::try_from(u32::from_le_bytes([l0, l1, l2, l3])).map_err(|_| "bad length")?;
    if length > MAX_VALUE_LEN {
        return Err("length exceeds limit");
    }
    if length != payload.len() {
        return Err("length mismatch (truncated or trailing data)");
    }
    let prefix = bytes.get(..12).ok_or("truncated header")?;
    if checksum(prefix, payload) != u32::from_le_bytes([c0, c1, c2, c3]) {
        return Err("checksum mismatch");
    }
    Ok(payload)
}

/// Checks the size limit and returns the length as stored in the header.
pub(crate) fn checked_length(value: &[u8]) -> crate::Result<u32> {
    let too_large = || crate::Error::TooLarge {
        len: value.len(),
        max: MAX_VALUE_LEN,
    };
    if value.len() > MAX_VALUE_LEN {
        return Err(too_large());
    }
    u32::try_from(value.len()).map_err(|_| too_large())
}
