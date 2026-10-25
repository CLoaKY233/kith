//! Tag-length-value extensions for forward compatibility.
//!
//! A message can end with an [`Extensions`] list. Readers look up the tags they know and
//! ignore the rest, so new optional fields can be added without a version bump. Tags must
//! be strictly increasing, which keeps the encoding canonical.

use alloc::vec::Vec;

use serde::{Deserialize, Deserializer, Serialize};

use crate::WireError;

/// One extension: a numeric tag and opaque bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extension {
    /// What the value means. Assigned per protocol.
    pub tag: u16,
    /// The encoded value.
    pub value: Vec<u8>,
}

/// A canonical list of extensions: tags strictly increasing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(transparent)]
pub struct Extensions(Vec<Extension>);

impl Extensions {
    /// No extensions.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Builds a list, rejecting duplicate or unordered tags.
    pub fn from_sorted(extensions: Vec<Extension>) -> Result<Self, WireError> {
        let ordered = extensions
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if a.tag < b.tag));
        if ordered {
            Ok(Self(extensions))
        } else {
            Err(WireError::UnorderedExtensions)
        }
    }

    /// Inserts or replaces the value for `tag`, keeping the list ordered.
    pub fn insert(&mut self, tag: u16, value: Vec<u8>) {
        match self.0.binary_search_by_key(&tag, |extension| extension.tag) {
            Ok(index) => {
                if let Some(slot) = self.0.get_mut(index) {
                    slot.value = value;
                }
            }
            Err(index) => self.0.insert(index, Extension { tag, value }),
        }
    }

    /// The value for `tag`, if present.
    #[must_use]
    pub fn get(&self, tag: u16) -> Option<&[u8]> {
        self.0
            .binary_search_by_key(&tag, |extension| extension.tag)
            .ok()
            .and_then(|index| self.0.get(index))
            .map(|extension| extension.value.as_slice())
    }

    /// Whether there are no extensions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// All extensions, in tag order.
    pub fn iter(&self) -> impl Iterator<Item = &Extension> {
        self.0.iter()
    }
}

impl<'de> Deserialize<'de> for Extensions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let extensions = Vec::<Extension>::deserialize(deserializer)?;
        Self::from_sorted(extensions).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{Extension, Extensions};
    use crate::{WireError, envelope};

    #[test]
    fn insert_keeps_order_and_replaces() {
        let mut ext = Extensions::new();
        ext.insert(9, vec![9]);
        ext.insert(1, vec![1]);
        ext.insert(9, vec![10]);
        assert_eq!(ext.get(1), Some(&[1][..]));
        assert_eq!(ext.get(9), Some(&[10][..]));
        assert_eq!(ext.get(5), None);
        let tags: Vec<u16> = ext.iter().map(|e| e.tag).collect();
        assert_eq!(tags, [1, 9]);
    }

    #[test]
    fn decoding_rejects_non_canonical_lists() {
        let unordered = vec![
            Extension {
                tag: 2,
                value: vec![],
            },
            Extension {
                tag: 1,
                value: vec![],
            },
        ];
        assert_eq!(
            Extensions::from_sorted(unordered.clone()),
            Err(WireError::UnorderedExtensions)
        );
        let bytes = postcard::to_allocvec(&unordered).unwrap();
        assert_eq!(
            envelope::decode_body::<Extensions>(&bytes),
            Err(WireError::Decode)
        );
    }
}
