//! Eden 0.2.1 ParamPackage string encoding, without Qt INI quoting or input translation.
//! Reference: Eden commit 58c1e20ee58efa3900ba616207d460886214480b,
//! src/common/param_package.cpp and src/common/string_util.cpp (SplitString).

use crate::fusion_source::errors::{ErrorCode, FusionError};
use std::collections::BTreeMap;

// Host limits for this codec, not claims about Eden's parser limits.
const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 16 * 1024;

pub(super) fn encode(entries: &[(&str, &str)]) -> Result<String, FusionError> {
    if entries.len() > MAX_ENTRIES {
        return Err(invalid("Too many ParamPackage fields."));
    }
    if entries.is_empty() {
        return Ok("[empty]".into());
    }

    let mut sorted = BTreeMap::new();
    for &(key, value) in entries {
        // Eden splits with getline: a trailing empty value after ':' is lost.
        for field in [key, value] {
            if field.len() > MAX_BYTES {
                return Err(invalid("ParamPackage exceeds the encoded byte limit."));
            }
            if field.is_empty() || field.chars().any(char::is_control) {
                return Err(invalid(
                    "ParamPackage fields must be nonempty and contain no control characters.",
                ));
            }
        }
        if sorted.insert(key, value).is_some() {
            return Err(invalid("Duplicate ParamPackage key."));
        }
    }

    let mut output = String::new();
    for (key, value) in sorted {
        if !output.is_empty() {
            output.push(',');
        }
        push_escaped(&mut output, key)?;
        output.push(':');
        push_escaped(&mut output, value)?;
    }
    Ok(output)
}

fn push_escaped(output: &mut String, field: &str) -> Result<(), FusionError> {
    for ch in field.chars() {
        match ch {
            ':' => output.push_str("$0"),
            ',' => output.push_str("$1"),
            '$' => output.push_str("$2"),
            _ => output.push(ch),
        }
        if output.len() > MAX_BYTES {
            return Err(invalid("ParamPackage exceeds the encoded byte limit."));
        }
    }
    Ok(())
}

fn invalid(message: &'static str) -> FusionError {
    // Parameter data may eventually be local device values; never echo it.
    FusionError::new(ErrorCode::InvalidCapability, message)
}
