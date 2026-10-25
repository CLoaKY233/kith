//! Range fingerprints.

use kith_crypto::{Context, Hash};
use kith_types::OpId;
use serde::{Deserialize, Serialize};

/// Domain separation for range fingerprints.
pub(crate) const FINGERPRINT_CONTEXT: Context = Context::new("kith 2026-10 sync fingerprint v1");

/// A digest of a set of op ids; see the crate docs for why ids are summed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Fingerprint(pub(crate) [u8; 32]);

/// Fingerprints a set of distinct ids: BLAKE3 under [`FINGERPRINT_CONTEXT`] of the count
/// (`u64`, little-endian) followed by the sum of the ids mod 2^256, each read as a
/// little-endian integer. The order of `ids` does not matter.
pub(crate) fn fingerprint(ids: &[OpId]) -> Fingerprint {
    let mut sum = [0_u64; 4];
    for id in ids {
        add_assign(&mut sum, id);
    }
    let mut input = [0_u8; 40];
    let (count, rest) = input.split_at_mut(8);
    count.copy_from_slice(&u64::try_from(ids.len()).unwrap_or(u64::MAX).to_le_bytes());
    for (out, limb) in rest.as_chunks_mut::<8>().0.iter_mut().zip(sum) {
        out.copy_from_slice(&limb.to_le_bytes());
    }
    Fingerprint(Hash::of(FINGERPRINT_CONTEXT, &input).to_bytes())
}

/// `sum += id` mod 2^256, with `sum` as four little-endian 64-bit limbs.
fn add_assign(sum: &mut [u64; 4], id: &OpId) {
    let mut carry = false;
    for (limb, chunk) in sum.iter_mut().zip(id.as_bytes().as_chunks::<8>().0) {
        let word = u64::from_le_bytes(*chunk);
        let (partial, first_carry) = limb.overflowing_add(word);
        let (total, second_carry) = partial.overflowing_add(u64::from(carry));
        *limb = total;
        carry = first_carry || second_carry;
    }
}
