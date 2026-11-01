//! IPC types as the page sees them: ids as hex, enum names in `snake_case`.
//!
//! The IPC types serialize ids as byte arrays and enums in serde's external tagging, which
//! suits postcard but not a web page. Rather than mirror every type, the serde output is
//! reshaped, so new IPC fields reach the page without changes here.

use core::fmt::Write as _;
use std::collections::BTreeMap;

use kith_ipc::{DnsRecord, Entry, RecordInfo, Status};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// Field names that hold a network or node id.
const ID_FIELDS: &[&str] = &["id", "network", "node"];
/// Field names that hold a `PathKind`.
const PATH_FIELDS: &[&str] = &["path", "from", "to"];

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

fn byte_hex(value: &Value) -> Option<String> {
    let items = value.as_array()?;
    if items.len() != 32 {
        return None;
    }
    let mut out = String::with_capacity(64);
    for item in items {
        let byte = u8::try_from(item.as_u64()?).ok()?;
        write!(out, "{byte:02x}").ok()?;
    }
    Some(out)
}

fn reshape(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if ID_FIELDS.contains(&key.as_str())
                    && let Some(hex) = byte_hex(field)
                {
                    *field = Value::String(hex);
                } else if PATH_FIELDS.contains(&key.as_str())
                    && let Value::String(text) = field
                {
                    *text = snake_case(text);
                } else {
                    reshape(field);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(reshape),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Any IPC value, reshaped for the page.
pub(crate) fn to_value(value: &impl Serialize) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(value)?;
    reshape(&mut value);
    Ok(value)
}

/// [`Status`], plus `admin`: network id to whether this device may change it.
pub(crate) fn status(status: &Status) -> Result<Value, serde_json::Error> {
    let admin: BTreeMap<String, bool> = status
        .networks
        .iter()
        .map(|network| (network.id.to_string(), network.admin))
        .collect();
    let mut value = to_value(status)?;
    if let Value::Object(map) = &mut value {
        map.insert("admin".to_owned(), serde_json::to_value(admin)?);
    }
    Ok(value)
}

/// A record as `{name, type, value, device}`.
pub(crate) fn record(record: &RecordInfo) -> Value {
    let (kind, data) = match &record.record {
        DnsRecord::A(address) => ("A", address.to_string()),
        DnsRecord::Aaaa(address) => ("AAAA", address.to_string()),
        DnsRecord::Cname(target) => ("CNAME", target.clone()),
        DnsRecord::Txt(text) => ("TXT", text.clone()),
        // DnsRecord is non_exhaustive; a future kind shows as its debug text.
        other => ("OTHER", format!("{other:?}")),
    };
    json!({"name": record.name, "type": kind, "value": data, "device": record.device})
}

/// A journal entry flattened to `{seq, at_ms, kind, ...fields}`, and its kind's name.
pub(crate) fn entry(entry: &Entry) -> Result<(String, Value), serde_json::Error> {
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
    out.insert("kind".to_owned(), Value::String(kind.clone()));
    Ok((kind, Value::Object(out)))
}
