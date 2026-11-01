//! `kith-ipc` values as the page sees them: ids as hex, path kinds and entry kinds in
//! `snake_case`, journal entries flat.
//!
//! The IPC types serialize ids as byte arrays and enums in serde's external tagging, which
//! suits postcard but not a web page. Rather than mirror every type, the serde output is
//! reshaped, so new IPC fields reach the page without changes here. [`unshape`] undoes it,
//! so a fixture written in the page's shape can be read back as IPC values.

use core::fmt::Write as _;

use kith_ipc::Entry;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// Field names whose string value may be a `PathKind`.
const PATH_FIELDS: &[&str] = &["path", "from", "to"];
/// `PathKind` variants as serde writes them, and as the page reads them.
const PATH_KINDS: &[(&str, &str)] = &[
    ("Direct", "direct"),
    ("Relay", "relay"),
    ("Stealth", "stealth"),
];
/// Bytes in a network or node id.
const ID_LEN: usize = 32;

/// `PeerOnline` as `peer_online`.
pub(crate) fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, character) in name.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(character.to_ascii_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}

/// `peer_online` as `PeerOnline`.
fn pascal_case(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                let mut word = first.to_ascii_uppercase().to_string();
                word.push_str(characters.as_str());
                word
            })
        })
        .collect()
}

/// A 32-byte array as lowercase hex; ids are the only such arrays in the IPC types.
fn id_hex(value: &Value) -> Option<String> {
    let items = value.as_array()?;
    if items.len() != ID_LEN {
        return None;
    }
    let mut out = String::with_capacity(ID_LEN * 2);
    for item in items {
        let byte = u8::try_from(item.as_u64()?).ok()?;
        write!(out, "{byte:02x}").ok()?;
    }
    Some(out)
}

/// Lowercase hex of an id back to its byte array.
fn id_bytes(text: &str) -> Option<Value> {
    if text.len() != ID_LEN * 2
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let bytes = text
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            core::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .map(Value::from)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Value::Array(bytes))
}

fn reshape(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if let Some(hex) = id_hex(field) {
                    *field = Value::String(hex);
                } else if let Value::String(text) = field
                    && PATH_FIELDS.contains(&key.as_str())
                    && let Some((_, page)) = PATH_KINDS.iter().find(|(ipc, _)| ipc == text)
                {
                    *text = (*page).to_owned();
                } else {
                    reshape(field);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(reshape),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn unreshape(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if let Value::String(text) = field {
                    if let Some(bytes) = id_bytes(text) {
                        *field = bytes;
                    } else if PATH_FIELDS.contains(&key.as_str())
                        && let Some((ipc, _)) = PATH_KINDS.iter().find(|(_, page)| page == text)
                    {
                        *text = (*ipc).to_owned();
                    }
                } else {
                    unreshape(field);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(unreshape),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Any IPC value, reshaped for the page. An [`Entry`] goes through [`entry`] instead.
pub fn to_value(value: &impl Serialize) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(value)?;
    reshape(&mut value);
    Ok(value)
}

/// A journal entry flattened to `{seq, at_ms, kind, ...fields}`.
pub fn entry(entry: &Entry) -> Result<Value, serde_json::Error> {
    let mut out = Map::new();
    out.insert("seq".to_owned(), entry.seq.into());
    out.insert("at_ms".to_owned(), entry.at_ms.into());
    let kind = match to_value(&entry.kind)? {
        Value::String(name) => snake_case(&name),
        Value::Object(tagged) => {
            let mut name = String::new();
            for (variant, fields) in tagged {
                name = snake_case(&variant);
                if let Value::Object(fields) = fields {
                    out.extend(fields);
                }
            }
            name
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => String::new(),
    };
    out.insert("kind".to_owned(), Value::String(kind));
    Ok(Value::Object(out))
}

/// Entries flattened as [`entry`] does.
pub fn entries(entries: &[Entry]) -> Result<Value, serde_json::Error> {
    entries
        .iter()
        .map(entry)
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

/// A value in the page's shape read back as an IPC type.
pub fn unshape<T: DeserializeOwned>(value: &Value) -> Result<T, serde_json::Error> {
    let mut value = value.clone();
    unreshape(&mut value);
    serde_json::from_value(value)
}

/// A flattened entry read back as an [`Entry`].
pub fn unshape_entry(value: &Value) -> Result<Entry, serde_json::Error> {
    let mut fields = value.as_object().cloned().unwrap_or_default();
    let seq = fields.remove("seq").unwrap_or(Value::Null);
    let at_ms = fields.remove("at_ms").unwrap_or(Value::Null);
    let kind = match fields.remove("kind") {
        Some(Value::String(kind)) => pascal_case(&kind),
        Some(_) | None => String::new(),
    };
    let mut tagged = Map::new();
    tagged.insert(kind, Value::Object(fields));
    let mut flat = Map::new();
    flat.insert("seq".to_owned(), seq);
    flat.insert("at_ms".to_owned(), at_ms);
    flat.insert("kind".to_owned(), Value::Object(tagged));
    unshape(&Value::Object(flat))
}

/// Serde for `kith_ipc::PathKind` as the page writes it: `direct`, `relay`, `stealth`.
pub(crate) mod path_kind {
    use kith_ipc::PathKind;
    use serde::de::Error as _;
    use serde::{Deserialize as _, Deserializer, Serializer};

    /// Writes the kind in `snake_case`.
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `with` passes a reference"
    )]
    pub(crate) fn serialize<S: Serializer>(kind: &PathKind, out: S) -> Result<S::Ok, S::Error> {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "PathKind is non_exhaustive; a new kind is shown as direct until named here"
        )]
        let name = match kind {
            PathKind::Relay => "relay",
            PathKind::Stealth => "stealth",
            _ => "direct",
        };
        out.serialize_str(name)
    }

    /// Reads a `snake_case` kind.
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<PathKind, D::Error> {
        match String::deserialize(input)?.as_str() {
            "direct" => Ok(PathKind::Direct),
            "relay" => Ok(PathKind::Relay),
            "stealth" => Ok(PathKind::Stealth),
            other => Err(D::Error::custom(format!("unknown path kind `{other}`"))),
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use kith_ipc::{EntryKind, PathKind};
    use kith_types::{NetworkId, NodeId};

    use super::*;

    #[test]
    fn entries_round_trip_through_the_page_shape() -> Result<(), serde_json::Error> {
        let original = Entry {
            seq: 4,
            at_ms: 1_700_000_000_000,
            kind: EntryKind::PathChanged {
                network: NetworkId::from_bytes([0xab; 32]),
                node: NodeId::from_bytes([0x01; 32]),
                name: "phone".into(),
                from: Some(PathKind::Direct),
                to: Some(PathKind::Relay),
            },
        };
        let page = entry(&original)?;
        assert_eq!(page["kind"], "path_changed");
        assert_eq!(page["network"], "ab".repeat(32));
        assert_eq!(page["to"], "relay");
        assert_eq!(unshape_entry(&page)?, original);
        Ok(())
    }

    #[test]
    fn blocked_ids_named_from_and_to_become_hex() -> Result<(), serde_json::Error> {
        let page = entry(&Entry {
            seq: 1,
            at_ms: 2,
            kind: EntryKind::Blocked {
                network: NetworkId::from_bytes([0xab; 32]),
                from: NodeId::from_bytes([0x02; 32]),
                from_name: "phone".into(),
                to: NodeId::from_bytes([0x03; 32]),
                to_name: "nas".into(),
                protocol: "tcp".into(),
                port: Some(22),
                count: 4,
            },
        })?;
        assert_eq!(page["from"], "02".repeat(32));
        assert_eq!(page["to"], "03".repeat(32));
        Ok(())
    }
}
