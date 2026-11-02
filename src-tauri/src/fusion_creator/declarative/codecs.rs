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
        || !config.preserve_unmanaged
        || config.keys.len() > 128
        || config.keys.iter().enumerate().any(|(i, key)| {
            config.keys[..i]
                .iter()
                .any(|other| other.id == key.id || other.address == key.address)
        })
    {
        return Err(invalid("Invalid or oversized config."));
    }
    let selected = validate_settings(config, settings)?;
    if config.format == ProgramConfigFormat::XmlAttributes {
        return render_xml(config, existing, selected);
    }
    let mut assignments = BTreeMap::new();
    if config.format != ProgramConfigFormat::Ini
        || !matches!(config.seed, ProgramConfigSeed::EmptyIni {})
    {
        return Err(invalid("Unsupported config format/seed."));
    }
    for (key, setting) in config.keys.iter().zip(selected) {
        let ProgramConfigAddress::Ini { section, key } = &key.address else {
            return Err(invalid("Config address does not match codec."));
        };
        if !ini_name(section) || !ini_name(key) {
            return Err(invalid("Invalid INI address."));
        }
        assignments.insert(
            (section.clone(), key.clone()),
            scalar_literal(&setting.value, true),
        );
    }
    let output = render_ini(existing.unwrap_or_default(), assignments)?;
    if output.len() > 2 * 1024 * 1024 {
        return Err(invalid("Oversized rendered config."));
    }
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
        let setting = selected
            .get(key.id.as_str())
            .ok_or_else(|| invalid("Missing config setting."))?;
        if key.allowed_values.is_empty()
            || key.allowed_values.len() > 64
            || key
                .allowed_values
                .iter()
                .any(|v| !typed_scalar(v, key.scalar_type))
            || !typed_scalar(&setting.value, key.scalar_type)
            || !key
                .allowed_values
                .iter()
                .any(|v| scalar_equal(v, &setting.value))
            || key
                .allowed_values
                .iter()
                .enumerate()
                .any(|(i, v)| key.allowed_values[..i].iter().any(|a| scalar_equal(a, v)))
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
        (ProgramScalar::Number(n), ProgramScalarType::Integer | ProgramScalarType::Number) => {
            n.as_f64().is_some_and(|v| {
                v.is_finite()
                    && v.abs() <= 9_007_199_254_740_991.0
                    && (scalar_type != ProgramScalarType::Integer || v.fract() == 0.0)
            })
        }
        (ProgramScalar::String(s), ProgramScalarType::String) => {
            s.chars().count() <= 2048 && !s.chars().any(char::is_control)
        }
        _ => false,
    }
}

fn ini_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.trim() == name
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b' ' | b'/' | b'-' | b'\\')
        })
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
    let mut sections = vec![IniSection {
        name: String::new(),
        header: String::new(),
        lines: Vec::new(),
    }];
    for line in input.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            sections.push(IniSection {
                name: name.to_owned(),
                header: line.to_owned(),
                lines: Vec::new(),
            });
        } else {
            sections.last_mut().unwrap().lines.push(line.to_owned());
        }
    }
    for (name, key) in assignments.keys() {
        let matching: Vec<_> = sections.iter().filter(|s| &s.name == name).collect();
        if matching.len() > 1
            || matching
                .iter()
                .flat_map(|s| &s.lines)
                .filter(|line| !line.trim_start().starts_with([';', '#']))
                .filter(|line| line.split_once('=').is_some_and(|(k, _)| k.trim() == key))
                .count()
                > 1
        {
            return Err(invalid("Ambiguous addressed INI section/key."));
        }
    }
    let mut output = String::new();
    for section in &sections {
        output.push_str(&section.header);
        for line in &section.lines {
            if let Some((key, _)) = line.split_once('=') {
                if let Some(value) =
                    assignments.remove(&(section.name.clone(), key.trim().to_owned()))
                {
                    output.push_str(key);
                    output.push('=');
                    output.push_str(&value);
                    if line.ends_with("\r\n") {
                        output.push_str("\r\n");
                    } else if line.ends_with('\n') {
                        output.push('\n');
                    }
                    continue;
                }
            }
            output.push_str(line);
        }
        let pending: Vec<_> = assignments
            .keys()
            .filter(|(name, _)| *name == section.name)
            .cloned()
            .collect();
        for address in pending {
            append_ini_line(
                &mut output,
                &format!("{}={}", address.1, assignments.remove(&address).unwrap()),
                newline,
            );
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
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str(newline);
    }
    output.push_str(line);
    output.push_str(newline);
}

// This is deliberately a small XML subset, not a general XML processor. Raw
// lexical spans retain private data; only addressed attribute values are edited.
const MAX_CONFIG_BYTES: usize = 2 * 1024 * 1024;
const MAX_XML_DEPTH: usize = 32;
const MAX_XML_NODES: usize = 16_384;
const MAX_XML_ATTRIBUTES: usize = 128;
const MAX_XML_TOTAL_ATTRIBUTES: usize = 65_536;

fn xml_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "xmlns"
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_value(value: &str) -> Result<(), FusionError> {
    if value.contains('<') || value.contains("]]>") || !value.chars().all(xml_char) {
        return Err(invalid("Invalid XML text/attribute."));
    }
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        rest = &rest[index..];
        let entity = ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"]
            .into_iter()
            .find(|entity| rest.starts_with(entity))
            .ok_or_else(|| invalid("Unsupported XML entity."))?;
        rest = &rest[entity.len()..];
    }
    Ok(())
}

struct XmlAttribute {
    name: String,
    start: usize,
    end: usize,
}

enum XmlContent {
    Element(XmlElement),
    Raw(String),
}

struct XmlElement {
    name: String,
    open: String,
    close: String,
    attributes: Vec<XmlAttribute>,
    changes: BTreeMap<String, String>,
    children: Vec<XmlContent>,
    self_closing: bool,
}

impl XmlElement {
    fn empty(name: &str) -> Self {
        Self {
            name: name.into(),
            open: format!("<{name}/>"),
            close: String::new(),
            attributes: Vec::new(),
            changes: BTreeMap::new(),
            children: Vec::new(),
            self_closing: true,
        }
    }

    fn target(&mut self, path: &[String]) -> Result<&mut Self, FusionError> {
        let Some(name) = path.first() else {
            return Ok(self);
        };
        let matching: Vec<_> = self
            .children
            .iter()
            .enumerate()
            .filter_map(|(i, child)| match child {
                XmlContent::Element(node) if &node.name == name => Some(i),
                _ => None,
            })
            .collect();
        if matching.len() > 1 {
            return Err(invalid("Ambiguous addressed XML child."));
        }
        let index = if let Some(index) = matching.first() {
            *index
        } else {
            self.children.push(XmlContent::Element(Self::empty(name)));
            self.children.len() - 1
        };
        let XmlContent::Element(node) = &mut self.children[index] else {
            unreachable!()
        };
        node.target(&path[1..])
    }

    fn write(&self, output: &mut String) {
        let mut open = self.open.clone();
        // Reverse lexical order leaves all saved byte ranges valid.
        for attr in self.attributes.iter().rev() {
            if let Some(value) = self.changes.get(&attr.name) {
                open.replace_range(attr.start..attr.end, &xml_escape(value));
            }
        }
        let mut additions = String::new();
        for (name, value) in &self.changes {
            if !self.attributes.iter().any(|a| &a.name == name) {
                additions.push_str(&format!(" {name}=\"{}\"", xml_escape(value)));
            }
        }
        let suffix = if self.self_closing { 2 } else { 1 };
        open.insert_str(open.len() - suffix, &additions);
        if self.self_closing && !self.children.is_empty() {
            open.truncate(open.len() - 2);
            open.push('>');
        }
        output.push_str(&open);
        for child in &self.children {
            match child {
                XmlContent::Raw(raw) => output.push_str(raw),
                XmlContent::Element(node) => node.write(output),
            }
        }
        if self.self_closing && !self.children.is_empty() {
            output.push_str(&format!("</{}>", self.name));
        } else {
            output.push_str(&self.close);
        }
    }
}

struct XmlParser<'a> {
    input: &'a str,
    pos: usize,
    nodes: usize,
    attributes: usize,
}

impl<'a> XmlParser<'a> {
    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }
    fn take(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }
    fn whitespace(&mut self) {
        while self
            .input
            .as_bytes()
            .get(self.pos)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.pos += 1;
        }
    }
    fn name(&mut self) -> Result<String, FusionError> {
        let start = self.pos;
        while self
            .input
            .as_bytes()
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        {
            self.pos += 1;
        }
        let name = &self.input[start..self.pos];
        if !xml_name(name) {
            return Err(invalid("Unsupported XML name/namespace."));
        }
        Ok(name.into())
    }
    fn attribute(&mut self, base: usize) -> Result<XmlAttribute, FusionError> {
        let name = self.name()?;
        self.whitespace();
        if !self.take("=") {
            return Err(invalid("Malformed XML attribute."));
        }
        self.whitespace();
        let quote = if self.take("\"") {
            '"'
        } else if self.take("'") {
            '\''
        } else {
            return Err(invalid("Unquoted XML attribute."));
        };
        let start = self.pos;
        let length = self
            .rest()
            .find(quote)
            .ok_or_else(|| invalid("Unclosed XML attribute."))?;
        self.pos += length;
        xml_value(&self.input[start..self.pos])?;
        let end = self.pos;
        self.pos += 1;
        self.attributes += 1;
        if self.attributes > MAX_XML_TOTAL_ATTRIBUTES {
            return Err(invalid("Too many XML attributes."));
        }
        Ok(XmlAttribute {
            name,
            start: start - base,
            end: end - base,
        })
    }
    fn comment(&mut self) -> Result<String, FusionError> {
        let start = self.pos;
        if !self.take("<!--") {
            return Err(invalid("Unsupported XML declaration."));
        }
        let length = self
            .rest()
            .find("-->")
            .ok_or_else(|| invalid("Unclosed XML comment."))?;
        let content = &self.rest()[..length];
        if content.contains("--") || content.ends_with('-') {
            return Err(invalid("Malformed XML comment."));
        }
        self.pos += length + 3;
        self.nodes += 1;
        if self.nodes > MAX_XML_NODES {
            return Err(invalid("Too many XML nodes."));
        }
        Ok(self.input[start..self.pos].into())
    }
    fn misc(&mut self) -> Result<(), FusionError> {
        loop {
            self.whitespace();
            if self.rest().starts_with("<!--") {
                self.comment()?;
            } else {
                return Ok(());
            }
        }
    }
    fn prolog(&mut self) -> Result<(), FusionError> {
        self.take("\u{feff}");
        if !self.take("<?xml") {
            return Ok(());
        }
        let mut attributes: Vec<XmlAttribute> = Vec::new();
        loop {
            let before = self.pos;
            self.whitespace();
            if self.take("?>") {
                break;
            }
            if before == self.pos || attributes.len() >= 3 {
                return Err(invalid("Malformed XML prolog."));
            }
            let attr = self.attribute(0)?;
            if attributes.iter().any(|a| a.name == attr.name) {
                return Err(invalid("Duplicate XML prolog attribute."));
            }
            let value = &self.input[attr.start..attr.end];
            let valid = match (attr.name.as_str(), attributes.len()) {
                ("version", 0) => value == "1.0",
                ("encoding", 1) => value.eq_ignore_ascii_case("UTF-8"),
                ("standalone", 1 | 2) => matches!(value, "yes" | "no"),
                _ => false,
            };
            if !valid {
                return Err(invalid("Unsupported XML prolog."));
            }
            attributes.push(attr);
        }
        if attributes.first().is_none_or(|a| a.name != "version") {
            return Err(invalid("Missing XML version."));
        }
        Ok(())
    }
    fn element(&mut self, depth: usize) -> Result<XmlElement, FusionError> {
        self.nodes += 1;
        if depth > MAX_XML_DEPTH || self.nodes > MAX_XML_NODES {
            return Err(invalid("XML tree exceeds bounds."));
        }
        let start = self.pos;
        if !self.take("<") {
            return Err(invalid("Expected XML element."));
        }
        let name = self.name()?;
        let mut attributes: Vec<XmlAttribute> = Vec::new();
        let self_closing;
        loop {
            let before = self.pos;
            self.whitespace();
            if self.take("/>") {
                self_closing = true;
                break;
            }
            if self.take(">") {
                self_closing = false;
                break;
            }
            if before == self.pos || attributes.len() >= MAX_XML_ATTRIBUTES {
                return Err(invalid("Malformed/bounded XML attributes."));
            }
            let attr = self.attribute(start)?;
            if attributes.iter().any(|a| a.name == attr.name) {
                return Err(invalid("Duplicate XML attribute."));
            }
            attributes.push(attr);
        }
        let mut node = XmlElement {
            name,
            open: self.input[start..self.pos].into(),
            close: String::new(),
            attributes,
            changes: BTreeMap::new(),
            children: Vec::new(),
            self_closing,
        };
        if self_closing {
            return Ok(node);
        }
        loop {
            if self.rest().starts_with("</") {
                let start = self.pos;
                self.pos += 2;
                if self.name()? != node.name {
                    return Err(invalid("Mismatched XML closing element."));
                }
                self.whitespace();
                if !self.take(">") {
                    return Err(invalid("Malformed XML closing element."));
                }
                node.close = self.input[start..self.pos].into();
                return Ok(node);
            } else if self.rest().starts_with("<!--") {
                node.children.push(XmlContent::Raw(self.comment()?));
            } else if self.rest().starts_with('<') {
                node.children
                    .push(XmlContent::Element(self.element(depth + 1)?));
            } else {
                let length = self
                    .rest()
                    .find('<')
                    .ok_or_else(|| invalid("Unclosed XML element."))?;
                let raw = &self.rest()[..length];
                xml_value(raw)?;
                self.nodes += 1;
                if self.nodes > MAX_XML_NODES {
                    return Err(invalid("Too many XML nodes."));
                }
                node.children.push(XmlContent::Raw(raw.into()));
                self.pos += length;
            }
        }
    }
}

fn parse_xml(input: &str) -> Result<(String, XmlElement, String), FusionError> {
    if input.len() > MAX_CONFIG_BYTES || !input.chars().all(xml_char) {
        return Err(invalid("Invalid/bounded XML document."));
    }
    let mut parser = XmlParser {
        input,
        pos: 0,
        nodes: 0,
        attributes: 0,
    };
    parser.prolog()?;
    parser.misc()?;
    let prefix = input[..parser.pos].into();
    let root = parser.element(1)?;
    let end = parser.pos;
    parser.misc()?;
    if parser.pos != input.len() {
        return Err(invalid("Multiple roots or unsupported XML content."));
    }
    Ok((prefix, root, input[end..].into()))
}

fn render_xml(
    config: &ProgramConfig,
    existing: Option<&str>,
    selected: Vec<&ProgramSetting>,
) -> Result<String, FusionError> {
    let ProgramConfigSeed::XmlRoot { root, attributes } = &config.seed else {
        return Err(invalid("XML requires root seed."));
    };
    if !xml_name(root) || attributes.len() > 16 {
        return Err(invalid("Invalid XML seed."));
    }
    let mut seed = XmlElement::empty(root);
    for attr in attributes {
        let kind = match attr.value {
            ProgramScalar::Boolean(_) => ProgramScalarType::Boolean,
            ProgramScalar::Number(_) => ProgramScalarType::Number,
            ProgramScalar::String(_) => ProgramScalarType::String,
        };
        if !xml_name(&attr.name)
            || !typed_scalar(&attr.value, kind)
            || seed.changes.contains_key(&attr.name)
        {
            return Err(invalid("Invalid/duplicate XML seed attribute."));
        }
        let literal = scalar_literal(&attr.value, false);
        if !literal.chars().all(xml_char) {
            return Err(invalid("Invalid XML seed scalar."));
        }
        seed.changes.insert(attr.name.clone(), literal);
    }
    let (prefix, mut node, suffix) = if let Some(input) = existing {
        parse_xml(input)?
    } else {
        (String::new(), seed, "\n".into())
    };
    if &node.name != root {
        return Err(invalid("XML root does not match seed."));
    }
    for (key, setting) in config.keys.iter().zip(selected) {
        let ProgramConfigAddress::XmlAttribute {
            elements,
            attribute,
        } = &key.address
        else {
            return Err(invalid("XML address required."));
        };
        if elements.len() > 8 || !elements.iter().all(|n| xml_name(n)) || !xml_name(attribute) {
            return Err(invalid("Invalid XML address."));
        }
        let literal = scalar_literal(&setting.value, false);
        if !literal.chars().all(xml_char) {
            return Err(invalid("Invalid XML scalar."));
        }
        node.target(elements)?
            .changes
            .insert(attribute.clone(), literal);
    }
    let mut output = prefix;
    node.write(&mut output);
    output.push_str(&suffix);
    if output.len() > MAX_CONFIG_BYTES {
        return Err(invalid("Oversized rendered XML."));
    }
    // Also enforce tree/attribute limits after missing nodes/attributes were added.
    parse_xml(&output)?;
    Ok(output)
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

#[cfg(test)]
#[path = "codecs_tests.rs"]
mod tests;
