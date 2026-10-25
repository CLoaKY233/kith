//! A small seeded generator: `SplitMix64` seeding into `xorshift64*`.

use core::time::Duration;

use kith_crypto::RandomError;
use kith_disco::RandomSource;

/// A deterministic pseudo-random generator. Not cryptographically secure.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator whose whole output is determined by `seed`.
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // xorshift has a fixed point at zero.
        Self { state: z.max(1) }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A uniform value in `0..bound`, or 0 when `bound` is 0.
    pub fn below(&mut self, bound: u64) -> u64 {
        let wide = (u128::from(self.next_u64()) * u128::from(bound)) >> 64;
        u64::try_from(wide).unwrap_or(0)
    }

    /// A uniform index into a collection of `len` items, or 0 when it is empty.
    pub fn index(&mut self, len: usize) -> usize {
        let bound = u64::try_from(len).unwrap_or(u64::MAX);
        usize::try_from(self.below(bound)).unwrap_or(0)
    }

    /// True with probability `per_mille / 1000`.
    pub fn chance(&mut self, per_mille: u16) -> bool {
        self.below(1000) < u64::from(per_mille)
    }

    /// A uniform duration in `0..max`.
    pub fn duration_below(&mut self, max: Duration) -> Duration {
        let nanos = u64::try_from(max.as_nanos()).unwrap_or(u64::MAX);
        Duration::from_nanos(self.below(nanos))
    }

    /// Fills `buffer` with random bytes.
    pub fn fill_bytes(&mut self, buffer: &mut [u8]) {
        for chunk in buffer.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(bytes.get(..chunk.len()).unwrap_or_default());
        }
    }
}

impl RandomSource for Rng {
    fn fill(&mut self, buffer: &mut [u8]) -> Result<(), RandomError> {
        self.fill_bytes(buffer);
        Ok(())
    }
}
