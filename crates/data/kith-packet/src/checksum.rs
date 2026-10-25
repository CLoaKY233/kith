//! The Internet checksum (RFC 1071) and its incremental update (RFC 1624).
//!
//! The accumulator sums 64-bit big-endian words with end-around carry. Because
//! `2^64 - 1` is a multiple of `2^16 - 1`, this is congruent to the classic
//! 16-bit ones'-complement sum but touches memory a quarter as often.

/// A running ones'-complement sum.
///
/// Feed it with [`add_bytes`](Self::add_bytes); only the *last* slice may have
/// an odd length, because an odd trailing byte is padded with zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Checksum {
    sum: u64,
}

impl Checksum {
    /// An empty sum.
    pub(crate) const fn new() -> Self {
        Self { sum: 0 }
    }

    /// Adds an integer value; congruent to adding its 16-bit digits.
    pub(crate) fn add_value(&mut self, value: u64) {
        let (sum, carry) = self.sum.overflowing_add(value);
        self.sum = sum.wrapping_add(u64::from(carry));
    }

    /// Adds `bytes` as big-endian 16-bit words, zero-padding an odd tail.
    pub(crate) fn add_bytes(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add_value(u64::from_be_bytes(*word));
        }
        let (pairs, tail) = rest.as_chunks::<2>();
        for pair in pairs {
            self.add_value(u64::from(u16::from_be_bytes(*pair)));
        }
        if let [last] = tail {
            self.add_value(u64::from(u16::from_be_bytes([*last, 0])));
        }
    }

    /// The folded 16-bit ones'-complement sum (not yet complemented).
    pub(crate) fn fold(self) -> u16 {
        let mut sum = self.sum;
        while sum > 0xffff {
            sum = (sum & 0xffff).wrapping_add(sum >> 16);
        }
        let [.., high, low] = sum.to_be_bytes();
        u16::from_be_bytes([high, low])
    }

    /// The checksum to store in a header: the complement of [`fold`](Self::fold).
    pub(crate) fn finish(self) -> u16 {
        !self.fold()
    }
}

/// The Internet checksum of `bytes` (RFC 1071), ready to store.
///
/// Over a region that already contains its checksum, the result is zero iff
/// the checksum is valid.
pub(crate) fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = Checksum::new();
    sum.add_bytes(bytes);
    sum.finish()
}

/// Updates `checksum` after one 16-bit word changed from `old` to `new`,
/// using RFC 1624 equation 3: `HC' = ~(~HC + ~m + m')`.
pub(crate) fn update_u16(checksum: u16, old: u16, new: u16) -> u16 {
    let mut sum = Checksum::new();
    sum.add_value(u64::from(!checksum));
    sum.add_value(u64::from(!old));
    sum.add_value(u64::from(new));
    sum.finish()
}

/// Updates `checksum` after a word-aligned field changed from `old` to `new`
/// (for example an address). Slices of different or odd length are ignored
/// pairwise beyond the shorter even prefix.
pub(crate) fn update_bytes(checksum: u16, old: &[u8], new: &[u8]) -> u16 {
    let (old_words, _) = old.as_chunks::<2>();
    let (new_words, _) = new.as_chunks::<2>();
    old_words
        .iter()
        .zip(new_words)
        .fold(checksum, |current, (before, after)| {
            update_u16(
                current,
                u16::from_be_bytes(*before),
                u16::from_be_bytes(*after),
            )
        })
}
