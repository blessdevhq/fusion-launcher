//! Pure bounded renderers for private, preserving declarative configurations.
use std::collections::BTreeMap;

use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::{
    ProgramConfig, ProgramConfigAddress, ProgramConfigFormat, ProgramConfigSeed, ProgramScalar,
    ProgramScalarType, ProgramSetting,
};

pub fn render_config(
    config: &ProgramConfig,
    existing: Option<&str>,
    settings: &[ProgramSetting],
) -> Result<String, FusionError> {
    if existing.is_some_and(|s| s.len() > 2 * 1024 * 1024)
        || !config.preserve_unmanaged || config.keys.len() > 128
        || config.keys.iter().enumerate().any(|(i, key)| config.keys[..i].iter().any(|other| other.id == key.id || other.address == key.address))
    {
        return Err(invalid("Invalid or oversized config."));
    }
    let selected = validate_settings(config, settings)?;
    if config.format == ProgramConfigFormat::XmlAttributes {
        return render_xml(config, existing, selected);
    }
    let mut assignments = BTreeMap::new();
    if config.format != ProgramConfigFormat::Ini || !matches!(config.seed, ProgramConfigSeed::EmptyIni {}) {
        return Err(invalid("Unsupported config format/seed."));
    }
    for (key, setting) in config.keys.iter().zip(selected) {
        let ProgramConfigAddress::Ini { section, key } = &key.address else {
            return Err(invalid("Config address does not match codec."));
        };
        if !ini_name(section) || !ini_name(key) {
            return Err(invalid("Invalid INI address."));
        }
        assignments.insert((section.clone(), key.clone()), scalar_literal(&setting.value, true));
    }
    let output = render_ini(existing.unwrap_or_default(), assignments)?;
    if output.len() > 2 * 1024 * 1024 { return Err(invalid("Oversized rendered config.")); }
    Ok(output)
}

fn validate_settings<'a>(
    config: &ProgramConfig,
    settings: &'a [ProgramSetting],
) -> Result<Vec<&'a ProgramSetting>, FusionError> {
    let mut selected = BTreeMap::new();
    for setting in settings.iter().filter(|s| s.config_id == config.id) {
        if !config.keys.iter().any(|k| k.id == setting.key_id)
            || selected.insert(setting.key_id.as_str(), setting).is_some()
        {
            return Err(invalid("Unknown or duplicate config setting."));
        }
    }
    let mut result = Vec::new();
    for key in &config.keys {
        let setting = selected.get(key.id.as_str())
            .ok_or_else(|| invalid("Missing config setting."))?;
        if key.allowed_values.is_empty() || key.allowed_values.len() > 64
            || key.allowed_values.iter().any(|v| !typed_scalar(v, key.scalar_type))
            || !typed_scalar(&setting.value, key.scalar_type)
            || !key.allowed_values.iter().any(|v| scalar_equal(v, &setting.value))
            || key.allowed_values.iter().enumerate().any(|(i, v)| key.allowed_values[..i].iter().any(|a| scalar_equal(a, v)))
        {
            return Err(invalid("Config setting violates its typed finite domain."));
        }
        result.push(*setting);
    }
    Ok(result)
}

fn scalar_equal(left: &ProgramScalar, right: &ProgramScalar) -> bool {
    match (left, right) {
        (ProgramScalar::Number(a), ProgramScalar::Number(b)) => a.as_f64() == b.as_f64(),
        _ => left == right,
    }
}

fn typed_scalar(value: &ProgramScalar, scalar_type: ProgramScalarType) -> bool {
    match (value, scalar_type) {
        (ProgramScalar::Boolean(_), ProgramScalarType::Boolean) => true,
        (ProgramScalar::Number(n), ProgramScalarType::Integer | ProgramScalarType::Number) => n.as_f64().is_some_and(|v| {
            v.is_finite() && v.abs() <= 9_007_199_254_740_991.0
                && (scalar_type != ProgramScalarType::Integer || v.fract() == 0.0)
        }),
        (ProgramScalar::String(s), ProgramScalarType::String) => s.chars().count() <= 2048 && !s.chars().any(char::is_control),
        _ => false,
    }
}

fn ini_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && name.trim() == name
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b' ' | b'/' | b'-' | b'\\'))
}

fn scalar_literal(value: &ProgramScalar, ini: bool) -> String {
    match value {
        ProgramScalar::Boolean(v) => v.to_string(),
        ProgramScalar::Number(v) => v.as_f64().unwrap().to_string(),
        ProgramScalar::String(v) if ini => serde_json::to_string(v).unwrap(),
        ProgramScalar::String(v) => v.clone(),
    }
}

struct IniSection {
    name: String,
    header: String,
    lines: Vec<String>,
}

fn render_ini(
    input: &str,
    mut assignments: BTreeMap<(String, String), String>,
) -> Result<String, FusionError> {
    let newline = if input.contains("\r\n") { "\r\n" } else { "\n" };
    let mut sections = vec![IniSection { name: String::new(), header: String::new(), lines: Vec::new() }];
    for line in input.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            sections.push(IniSection { name: name.to_owned(), header: line.to_owned(), lines: Vec::new() });
        } else {
            sections.last_mut().unwrap().lines.push(line.to_owned());
        }
    }
    for (name, key) in assignments.keys() {
        let matching: Vec<_> = sections.iter().filter(|s| &s.name == name).collect();
        if matching.len() > 1 || matching.iter().flat_map(|s| &s.lines)
            .filter(|line| !line.trim_start().starts_with([';', '#']))
            .filter(|line| line.split_once('=').is_some_and(|(k, _)| k.trim() == key)).count() > 1 {
            return Err(invalid("Ambiguous addressed INI section/key."));
        }
    }
    let mut output = String::new();
    for section in &sections {
        output.push_str(&section.header);
        for line in &section.lines {
            if let Some((key, _)) = line.split_once('=') {
                if let Some(value) = assignments.remove(&(section.name.clone(), key.trim().to_owned())) {
                    output.push_str(key);
                    output.push('=');
                    output.push_str(&value);
                    if line.ends_with("\r\n") { output.push_str("\r\n"); }
                    else if line.ends_with('\n') { output.push('\n'); }
                    continue;
                }
            }
            output.push_str(line);
        }
        let pending: Vec<_> = assignments.keys().filter(|(name, _)| *name == section.name).cloned().collect();
        for address in pending {
            append_ini_line(&mut output, &format!("{}={}", address.1, assignments.remove(&address).unwrap()), newline);
        }
    }
    let mut last = String::new();
    for ((section, key), value) in assignments {
        if last != section {
            append_ini_line(&mut output, &format!("[{section}]"), newline);
            last = section;
        }
        append_ini_line(&mut output, &format!("{key}={value}"), newline);
    }
    Ok(output)
}

fn append_ini_line(output: &mut String, line: &str, newline: &str) {
    if !output.is_empty() && !output.ends_with('\n') { output.push_str(newline); }
    output.push_str(line);
    output.push_str(newline);
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

#[cfg(test)]
#[path = "codecs_tests.rs"]
mod tests;
