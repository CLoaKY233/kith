//! Recovery codes: a network's recovery key, shown to its owner once as words.
//!
//! A code is 16 random bytes and a 2-byte checksum, one word per byte from a list of 256,
//! so 18 words. The Ed25519 recovery key is derived from the 16 bytes; the network stores
//! only its public half, and the words are kept nowhere.

use alloc::string::String;

use kith_crypto::{Hash, SigningKey, derive_key};
use zeroize::Zeroizing;

use crate::contexts;

/// Random bytes in a code.
const SECRET_LEN: usize = 16;

/// Checksum bytes after them.
const CHECKSUM_LEN: usize = 2;

/// Words in a code.
pub const RECOVERY_WORDS: usize = SECRET_LEN + CHECKSUM_LEN;

/// Why words are not a recovery code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecoveryError {
    /// The wrong number of words.
    #[error("a recovery code is {RECOVERY_WORDS} words, not {0}")]
    WrongLength(usize),
    /// A word is not in the list; its position counts from 1.
    #[error("word {0} of the recovery code is not one it can contain")]
    UnknownWord(usize),
    /// Every word is known, but they do not belong together: one is wrong or out of place.
    #[error("the recovery code does not check out; a word is wrong or out of place")]
    BadChecksum,
    /// The operating system's random source failed.
    #[error("random source unavailable")]
    Random,
}

/// A recovery code. Not `Clone`; zeroizes on drop.
pub struct RecoveryCode {
    secret: Zeroizing<[u8; SECRET_LEN]>,
}

impl RecoveryCode {
    /// Generates a fresh code from the operating system's random source.
    #[cfg(feature = "getrandom")]
    pub fn generate() -> Result<Self, RecoveryError> {
        let mut secret = Zeroizing::new([0_u8; SECRET_LEN]);
        kith_crypto::fill_random(secret.as_mut_slice()).map_err(|_| RecoveryError::Random)?;
        Ok(Self { secret })
    }

    /// Reads a code from its words, in any case, separated by spaces, commas or hyphens.
    pub fn parse(text: &str) -> Result<Self, RecoveryError> {
        let words = || {
            text.split(|c: char| c.is_whitespace() || c == ',' || c == '-')
                .filter(|word| !word.is_empty())
        };
        let count = words().count();
        if count != RECOVERY_WORDS {
            return Err(RecoveryError::WrongLength(count));
        }
        let mut bytes = Zeroizing::new([0_u8; RECOVERY_WORDS]);
        for (position, (slot, word)) in bytes.iter_mut().zip(words()).enumerate() {
            let unknown = RecoveryError::UnknownWord(position.saturating_add(1));
            let index = WORDS
                .iter()
                .position(|known| known.eq_ignore_ascii_case(word))
                .ok_or(unknown)?;
            *slot = u8::try_from(index).map_err(|_| unknown)?;
        }
        let mut secret = Zeroizing::new([0_u8; SECRET_LEN]);
        for (slot, byte) in secret.iter_mut().zip(bytes.iter()) {
            *slot = *byte;
        }
        let code = Self { secret };
        if bytes.get(SECRET_LEN..) == Some(code.checksum().as_slice()) {
            Ok(code)
        } else {
            Err(RecoveryError::BadChecksum)
        }
    }

    /// The code as lowercase words separated by spaces.
    pub fn to_words(&self) -> Zeroizing<String> {
        let mut text = Zeroizing::new(String::new());
        for byte in self.secret.iter().chain(self.checksum().iter()) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(WORDS.get(usize::from(*byte)).copied().unwrap_or_default());
        }
        text
    }

    /// The recovery key the code stands for.
    pub fn signing_key(&self) -> SigningKey {
        SigningKey::from_seed(&derive_key(contexts::RECOVERY_KEY, self.secret.as_slice()))
    }

    fn checksum(&self) -> [u8; CHECKSUM_LEN] {
        let hash = Hash::of(contexts::RECOVERY_CHECKSUM, self.secret.as_slice());
        let mut checksum = [0_u8; CHECKSUM_LEN];
        for (slot, byte) in checksum.iter_mut().zip(hash.as_bytes()) {
            *slot = *byte;
        }
        checksum
    }

    #[cfg(test)]
    fn from_secret(secret: [u8; SECRET_LEN]) -> Self {
        Self {
            secret: Zeroizing::new(secret),
        }
    }
}

impl core::fmt::Debug for RecoveryCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RecoveryCode(<redacted>)")
    }
}

/// One word per byte value: short, common, and distinct.
const WORDS: [&str; 256] = [
    "acorn", "actor", "agent", "alarm", "album", "amber", "angle", "apple", "arena", "arrow",
    "aspen", "atlas", "badge", "baker", "bamboo", "barn", "beach", "beard", "bench", "berry",
    "bison", "blade", "bloom", "boat", "brick", "broom", "brush", "cabin", "cable", "cactus",
    "camel", "candy", "canoe", "carpet", "castle", "cedar", "chalk", "cherry", "cloud", "clover",
    "comet", "coral", "cotton", "crane", "crown", "daisy", "dance", "desert", "donut", "dragon",
    "drum", "duck", "eagle", "echo", "elbow", "ember", "engine", "fabric", "falcon", "fern",
    "ferry", "finch", "flag", "flame", "flute", "foam", "forest", "fossil", "fox", "frog",
    "galaxy", "garden", "garlic", "gecko", "ginger", "glove", "goat", "gold", "gravel", "guitar",
    "hammer", "harbor", "harp", "hazel", "helmet", "heron", "hippo", "honey", "hotel", "igloo",
    "iris", "iron", "island", "ivory", "jacket", "jaguar", "jelly", "jewel", "juice", "jungle",
    "kayak", "kettle", "kiwi", "koala", "kite", "knot", "ladder", "lagoon", "lamp", "laser",
    "lava", "lemon", "lily", "lizard", "llama", "lotus", "magnet", "mango", "maple", "marble",
    "meadow", "melon", "mint", "mirror", "monkey", "moose", "muffin", "music", "napkin", "nectar",
    "needle", "nest", "noodle", "novel", "oasis", "ocean", "olive", "onion", "opal", "orbit",
    "otter", "owl", "oyster", "paddle", "palace", "panda", "paper", "parrot", "pasta", "peach",
    "pearl", "pebble", "pepper", "piano", "pickle", "pigeon", "pillow", "pilot", "pine", "pixel",
    "plum", "polar", "pony", "poppy", "puffin", "puzzle", "quail", "quartz", "quilt", "rabbit",
    "radar", "radish", "raft", "rain", "raven", "reef", "ribbon", "river", "robin", "rocket",
    "rose", "ruby", "saddle", "safari", "salad", "salmon", "saturn", "scarf", "seal", "shell",
    "silver", "sketch", "sled", "sloth", "snail", "socket", "sofa", "spider", "spoon", "squid",
    "statue", "sugar", "summit", "sunset", "swan", "table", "taco", "tango", "teapot", "tennis",
    "tiger", "timber", "toast", "tomato", "topaz", "torch", "tower", "tulip", "tunnel", "turtle",
    "tuxedo", "urban", "utopia", "valley", "velvet", "violet", "violin", "viper", "visor",
    "voyage", "waffle", "wagon", "walnut", "walrus", "wasabi", "water", "whale", "wheat", "willow",
    "window", "winter", "wizard", "wolf", "wombat", "yacht", "yak", "yarn", "yeti", "yogurt",
    "yoyo", "zebra", "zenith", "zigzag", "zinc", "zipper", "zone",
];

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::borrow::ToOwned;
    use alloc::string::String;
    use alloc::vec::Vec;

    use super::{RECOVERY_WORDS, RecoveryCode, RecoveryError, WORDS};

    #[test]
    fn words_round_trip_and_name_one_key() {
        let code = RecoveryCode::from_secret([0x5a; 16]);
        let words = code.to_words();
        assert_eq!(words.split(' ').count(), RECOVERY_WORDS);
        let shouted = words.to_uppercase().replace(' ', "-");
        let parsed = RecoveryCode::parse(&shouted).unwrap();
        assert_eq!(parsed.signing_key().node_id(), code.signing_key().node_id());
        assert_ne!(
            RecoveryCode::from_secret([0x5b; 16])
                .signing_key()
                .node_id(),
            code.signing_key().node_id()
        );
    }

    #[test]
    fn a_wrong_word_is_caught() {
        let secret = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let words: Vec<String> = RecoveryCode::from_secret(secret)
            .to_words()
            .split(' ')
            .map(str::to_owned)
            .collect();
        assert_eq!(
            RecoveryCode::parse(&words[..17].join(" ")).unwrap_err(),
            RecoveryError::WrongLength(17)
        );
        let mut unknown = words.clone();
        unknown[4] = "kith".into();
        assert_eq!(
            RecoveryCode::parse(&unknown.join(" ")).unwrap_err(),
            RecoveryError::UnknownWord(5)
        );
        let mut swapped = words;
        swapped.swap(0, 1);
        assert_eq!(
            RecoveryCode::parse(&swapped.join(" ")).unwrap_err(),
            RecoveryError::BadChecksum
        );
    }

    #[test]
    fn the_list_has_256_distinct_lowercase_words() {
        let mut sorted = WORDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 256);
        assert!(
            WORDS
                .iter()
                .all(|word| word.bytes().all(|b| b.is_ascii_lowercase()))
        );
    }
}
