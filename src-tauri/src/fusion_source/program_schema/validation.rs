//! Structural serde guards; no filesystem, network or execution authority.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Number, Value};

use crate::fusion_source::creator_schema::ComponentOrigin;
use crate::fusion_source::errors::{ErrorCode, FusionError};

pub const MAX_PROGRAM_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_REVISION: u64 = 9_007_199_254_740_991;

fn bad<E: de::Error>() -> E {
    E::custom("Value is outside the frozen program contract.")
}
fn length(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.chars().count())
}
pub(super) fn text<'de, D: Deserializer<'de>, const MIN: usize, const MAX: usize>(
    d: D,
) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if !length(&value, MIN, MAX) {
        return Err(bad());
    }
    Ok(value)
}
pub(super) fn list<
    'de,
    D: Deserializer<'de>,
    T: Deserialize<'de>,
    const MIN: usize,
    const MAX: usize,
>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    let values = Vec::<T>::deserialize(d)?;
    if !(MIN..=MAX).contains(&values.len()) {
        return Err(bad());
    }
    Ok(values)
}
pub(super) fn ids<'de, D: Deserializer<'de>, const MAX: usize>(
    d: D,
) -> Result<Vec<String>, D::Error> {
    let values = list::<D, String, 1, MAX>(d)?;
    let mut seen = BTreeSet::new();
    if values
        .iter()
        .any(|value| !length(value, 1, 128) || !seen.insert(value))
    {
        return Err(bad());
    }
    Ok(values)
}
fn valid_path(value: &str, root: bool) -> bool {
    if root && value.is_empty() {
        return true;
    }
    if !length(value, 1, 512)
        || value.starts_with('/')
        || value.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        || value.chars().any(char::is_control)
    {
        return false;
    }
    value.split('/').all(|part| {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        !part.is_empty()
            && !matches!(part, "." | "..")
            && !part.ends_with(['.', ' '])
            && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    })
}
pub(super) fn path<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if !valid_path(&value, false) {
        return Err(bad());
    }
    Ok(value)
}
pub(super) fn directory<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if !valid_path(&value, true) {
        return Err(bad());
    }
    Ok(value)
}
pub(super) fn directories<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let values = list::<D, String, 0, 64>(d)?;
    let mut seen = BTreeSet::new();
    if values
        .iter()
        .any(|value| !valid_path(value, false) || !seen.insert(value))
    {
        return Err(bad());
    }
    Ok(values)
}
pub(super) fn clean<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = text::<D, 0, 2048>(d)?;
    if value.chars().any(char::is_control) {
        return Err(bad());
    }
    Ok(value)
}
pub(super) fn prefix<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = text::<D, 0, 64>(d)?;
    if !value.is_empty() {
        let option = value
            .strip_prefix("--")
            .or_else(|| value.strip_prefix('-'))
            .and_then(|rest| rest.strip_suffix('='));
        if !option.is_some_and(|option| {
            !option.is_empty()
                && option
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        }) {
            return Err(bad());
        }
    }
    Ok(value)
}
pub(super) fn xml_name<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = text::<D, 1, 128>(d)?;
    if !value
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return Err(bad());
    }
    Ok(value)
}
pub(super) fn ini_name<'de, D: Deserializer<'de>, const KEY: bool>(
    d: D,
) -> Result<String, D::Error> {
    let value = text::<D, 1, 128>(d)?;
    if !value.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(b, b'_' | b'.' | b' ' | b'/' | b'-')
            || (KEY && b == b'\\')
    }) {
        return Err(bad());
    }
    Ok(value)
}
pub(super) fn xml_elements<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let values = list::<D, String, 0, 8>(d)?;
    for value in &values {
        if !length(value, 1, 128)
            || !value
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        {
            return Err(bad());
        }
    }
    Ok(values)
}
pub(super) fn environment<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = text::<D, 1, 64>(d)?;
    if !value.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
        || !value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        || matches!(
            value.as_str(),
            "PATH"
                | "PATHEXT"
                | "COMSPEC"
                | "SYSTEMROOT"
                | "WINDIR"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "USERPROFILE"
                | "HOME"
                | "TEMP"
                | "TMP"
                | "QT_PLUGIN_PATH"
                | "QT_QPA_PLATFORM_PLUGIN_PATH"
                | "PYTHONPATH"
                | "PYTHONHOME"
                | "NODE_OPTIONS"
                | "RUSTC_WRAPPER"
        )
    {
        return Err(bad());
    }
    Ok(value)
}
pub(super) fn revision<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let value = Number::deserialize(d)?;
    if let Some(n) = value.as_u64().filter(|n| *n <= MAX_REVISION) {
        return Ok(n);
    }
    if let Some(n) = value
        .as_f64()
        .filter(|n| *n >= 0.0 && *n <= MAX_REVISION as f64 && n.fract() == 0.0)
    {
        return Ok(n as u64);
    }
    Err(bad())
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
// URI syntax, not platform-specific Unicode whitespace or WHATWG IRI normalization.
static HTTPS_URI: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"\A[hH][tT][tT][pP][sS]://(?:[A-Za-z0-9._~!$&'()*+,;=:\[\]-]|%[A-Fa-f0-9]{2})+(?:/(?:[A-Za-z0-9._~!$&'()*+,;=:@/-]|%[A-Fa-f0-9]{2})*)?(?:\?(?:[A-Za-z0-9._~!$&'()*+,;=:@/?-]|%[A-Fa-f0-9]{2})*)?(?:#(?:[A-Za-z0-9._~!$&'()*+,;=:@/?-]|%[A-Fa-f0-9]{2})*)?\z",
    )
    .expect("Static HTTPS URI grammar")
});

pub(super) fn origin<'de, D: Deserializer<'de>>(d: D) -> Result<ComponentOrigin, D::Error> {
    let origin = ComponentOrigin::deserialize(d)?;
    let valid = match &origin {
        ComponentOrigin::Managed { artifact_ref, .. } => length(artifact_ref, 1, 128),
        ComponentOrigin::External {
            url,
            transport_blake3,
            inventory_blake3,
        } => {
            let parsed = url::Url::parse(url);
            let authority = url
                .split_once("://")
                .map(|(_, tail)| tail.split(['/', '?', '#']).next().unwrap_or_default());
            hash(transport_blake3)
                && hash(inventory_blake3)
                && HTTPS_URI.is_match(url)
                && authority.is_some_and(|a| !a.contains('@'))
                && parsed.is_ok_and(|url| {
                    url.scheme() == "https"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                })
        }
        ComponentOrigin::UserProvided {
            requirement_id,
            inventory_blake3,
            ..
        } => length(requirement_id, 1, 128) && hash(inventory_blake3),
    };
    if !valid {
        return Err(bad());
    }
    Ok(origin)
}

/// Detect duplicate keys at every depth before untagged serde or maps can erase them.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Number::from_f64(v)
                    .map(|v| UniqueValue(Value::Number(v)))
                    .ok_or_else(bad)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("Duplicate JSON key."));
                    }
                    let UniqueValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        d.deserialize_any(JsonVisitor)
    }
}
pub(crate) fn parse_unique_json(json: &str, max_bytes: usize) -> Result<Value, FusionError> {
    if json.len() > max_bytes {
        return Err(FusionError::new(
            ErrorCode::InvalidJson,
            "Metadata exceeds its byte limit.",
        ));
    }
    let mut decoder = serde_json::Deserializer::from_str(json);
    let UniqueValue(value) = UniqueValue::deserialize(&mut decoder).map_err(|_| {
        FusionError::new(
            ErrorCode::InvalidJson,
            "Invalid or duplicate-key JSON metadata.",
        )
    })?;
    decoder
        .end()
        .map_err(|_| FusionError::new(ErrorCode::InvalidJson, "Trailing JSON metadata."))?;
    Ok(value)
}

pub(super) fn bindings<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, super::ProgramLogicalBinding>, D::Error> {
    BTreeMap::deserialize(d)
}
