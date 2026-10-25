//! The sync wire format.

use alloc::vec::Vec;

use kith_ledger::MAX_SIGNED_OP_LEN;
use kith_types::{NetworkId, OpId};
use kith_wire::envelope;
use kith_wire::registry::SYNC_MESSAGE;
use serde::{Deserialize, Serialize};

use crate::{
    Fingerprint, MAX_HEADS, MAX_IDS_PER_MESSAGE, MAX_OPS_PER_MESSAGE, MAX_RANGE_ITEMS,
    MAX_SYNC_MESSAGE_LEN, SYNC_VERSION, SyncError,
};

/// The exclusive upper end of a range of op ids.
///
/// The derived order is the order of the id space: every `Id` sorts before `End`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Bound {
    /// Ids strictly below this one.
    Id(OpId),
    /// The end of the id space: every id is below it.
    End,
}

impl Bound {
    /// Whether `id` lies below this bound.
    pub fn is_above(&self, id: &OpId) -> bool {
        match self {
            Self::Id(bound) => id < bound,
            Self::End => true,
        }
    }
}

/// One range of a [`Message::Ranges`]. Its lower bound is the previous item's upper bound,
/// or the start of the id space for the first item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RangeItem {
    /// The sender's fingerprint of the range.
    Fingerprint {
        /// Where the range ends.
        upper_bound: Bound,
        /// The sender's fingerprint of its ids in the range.
        fingerprint: Fingerprint,
    },
    /// Every id the sender holds in the range, strictly ascending.
    IdList {
        /// Where the range ends.
        upper_bound: Bound,
        /// The ids, at most [`MAX_IDS_PER_MESSAGE`] across the whole message.
        ids: Vec<OpId>,
    },
    /// The range is in sync; nothing to do.
    Skip {
        /// Where the range ends.
        upper_bound: Bound,
    },
}

impl RangeItem {
    /// Where the range ends.
    pub const fn upper_bound(&self) -> Bound {
        match self {
            Self::Fingerprint { upper_bound, .. }
            | Self::IdList { upper_bound, .. }
            | Self::Skip { upper_bound } => *upper_bound,
        }
    }
}

/// A sync message. See the crate docs for the protocol.
///
/// Plain wire data: [`Message::decode`] checks every bound and the structure of ranges, so
/// a decoded message always satisfies the documented invariants. The variant order is part
/// of the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Opens the session: the network and the sender's DAG heads (at most [`MAX_HEADS`];
    /// empty if it has more).
    Hello {
        /// The network being synced.
        network: NetworkId,
        /// The sender's heads, sorted.
        heads: Vec<OpId>,
    },
    /// Range reconciliation: a partition of the whole id space, at most
    /// [`MAX_RANGE_ITEMS`] ranges, ending at [`Bound::End`]. Ends a flight.
    Ranges(Vec<RangeItem>),
    /// Op ids the sender lacks and asks for, at most [`MAX_IDS_PER_MESSAGE`].
    Want(Vec<OpId>),
    /// Signed ops as their exact bytes, at most [`MAX_OPS_PER_MESSAGE`].
    Ops(Vec<Vec<u8>>),
    /// The sender has no unresolved ranges. Ends a flight.
    Done,
}

impl Message {
    /// Encodes the message in its envelope, checking every bound first.
    pub fn encode(&self) -> Result<Vec<u8>, SyncError> {
        self.validate()?;
        let bytes = envelope::encode(SYNC_MESSAGE, SYNC_VERSION, self)?;
        if bytes.len() > MAX_SYNC_MESSAGE_LEN {
            return Err(SyncError::TooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    /// Decodes and validates a message received from a peer.
    pub fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        if bytes.len() > MAX_SYNC_MESSAGE_LEN {
            return Err(SyncError::TooLarge(bytes.len()));
        }
        let (version, message): (u64, Self) = envelope::decode(bytes, SYNC_MESSAGE, SYNC_VERSION)?;
        if version != SYNC_VERSION {
            return Err(SyncError::UnsupportedVersion(version));
        }
        message.validate()?;
        Ok(message)
    }

    /// Checks the bounds and, for ranges, the partition structure.
    fn validate(&self) -> Result<(), SyncError> {
        let within = |len: usize, max: usize| {
            if len <= max {
                Ok(())
            } else {
                Err(SyncError::LimitExceeded)
            }
        };
        match self {
            Self::Hello { heads, .. } => within(heads.len(), MAX_HEADS),
            Self::Ranges(items) => validate_ranges(items),
            Self::Want(ids) => within(ids.len(), MAX_IDS_PER_MESSAGE),
            Self::Ops(ops) => {
                within(ops.len(), MAX_OPS_PER_MESSAGE)?;
                ops.iter()
                    .try_for_each(|op| within(op.len(), MAX_SIGNED_OP_LEN))
            }
            Self::Done => Ok(()),
        }
    }
}

/// Checks that `items` partition the id space in order and that every id list is sorted
/// and inside its range.
fn validate_ranges(items: &[RangeItem]) -> Result<(), SyncError> {
    if items.is_empty() || items.len() > MAX_RANGE_ITEMS {
        return Err(SyncError::LimitExceeded);
    }
    let mut total_ids = 0_usize;
    let mut lower: Option<OpId> = None;
    let mut previous_upper: Option<Bound> = None;
    for item in items {
        let upper = item.upper_bound();
        if previous_upper.is_some_and(|previous| previous >= upper) {
            return Err(SyncError::MalformedRanges);
        }
        if let RangeItem::IdList { ids, .. } = item {
            total_ids = total_ids.saturating_add(ids.len());
            if total_ids > MAX_IDS_PER_MESSAGE {
                return Err(SyncError::LimitExceeded);
            }
            let sorted = ids.windows(2).all(|pair| pair.first() < pair.get(1));
            let above_lower = ids
                .first()
                .is_none_or(|first| lower.is_none_or(|lower| *first >= lower));
            let below_upper = ids.last().is_none_or(|last| upper.is_above(last));
            if !(sorted && above_lower && below_upper) {
                return Err(SyncError::MalformedRanges);
            }
        }
        lower = match upper {
            Bound::Id(id) => Some(id),
            Bound::End => None,
        };
        previous_upper = Some(upper);
    }
    if previous_upper == Some(Bound::End) {
        Ok(())
    } else {
        Err(SyncError::MalformedRanges)
    }
}
