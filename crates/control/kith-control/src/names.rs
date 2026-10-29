//! Unique device names within a network, derived from the ledger alone so every device
//! computes the same ones without coordinating.

use alloc::borrow::ToOwned;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use kith_ledger::Device;
use kith_types::NodeId;

/// Longest DNS label.
const MAX_LABEL: usize = 63;
/// Hex digits of the device id a clashing name starts with.
const SUFFIX_START: usize = 4;
/// Used when nothing of a requested name survives normalising.
const FALLBACK: &str = "device";

/// `name` as a DNS label: lowercase ASCII letters, digits and single hyphens, never
/// starting or ending with a hyphen, at most 63 bytes, `device` when nothing is left.
pub(crate) fn normalize(name: &str) -> String {
    let mut label = String::new();
    for ch in name.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            label.push(ch);
        } else if !label.is_empty() && !label.ends_with('-') {
            label.push('-');
        }
    }
    label.truncate(MAX_LABEL);
    let trimmed = label.trim_end_matches('-');
    if trimmed.is_empty() {
        FALLBACK.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// A unique label for every device. The earliest-admitted device keeps the plain name;
/// later ones with the same name get a suffix from their own id, grown until it clashes
/// with nothing, so a device's name does not shift when others join or leave.
pub(crate) fn assign(devices: &BTreeMap<NodeId, Device>) -> BTreeMap<NodeId, String> {
    let mut order: Vec<(&NodeId, &Device)> = devices.iter().collect();
    order.sort_by_key(|(node, device)| (device.admitted_at_lamport, device.admitted_by, **node));
    let bases: Vec<(NodeId, String)> = order
        .iter()
        .map(|(node, device)| (**node, normalize(&device.name)))
        .collect();
    let mut reserved: BTreeSet<String> = bases.iter().map(|(_, base)| base.clone()).collect();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let mut labels = BTreeMap::new();
    for (node, base) in bases {
        let label = if claimed.insert(base.clone()) {
            base
        } else {
            let label = suffixed(&base, &node, &reserved);
            reserved.insert(label.clone());
            claimed.insert(label.clone());
            label
        };
        labels.insert(node, label);
    }
    labels
}

fn suffixed(base: &str, node: &NodeId, taken: &BTreeSet<String>) -> String {
    let id = node.to_string();
    for digits in SUFFIX_START..=id.len() {
        let suffix = id.get(..digits).unwrap_or(&id);
        let room = MAX_LABEL.saturating_sub(suffix.len() + 1);
        let stem = base
            .get(..room.min(base.len()))
            .unwrap_or(base)
            .trim_end_matches('-');
        let label = format!("{stem}-{suffix}");
        if !taken.contains(&label) {
            return label;
        }
    }
    id
}
