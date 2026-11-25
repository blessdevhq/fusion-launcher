//! Built-in config templates. String variables belong inside double-quoted JSON/TOML
//! strings; numbers and addon flags can also be used as unquoted scalar values.
//! INI uses literal values or double-quoted strings, with no multiline values.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::schema::{ConfigFormat, ConfigTemplateSpec};

#[derive(Debug, Clone)]
pub struct TemplateContext {
    pub workspace_dir: PathBuf,
    pub appdata_dir: PathBuf,
    pub local_appdata_dir: PathBuf,
    pub display_width: u32,
    pub display_height: u32,
    pub refresh_rate: u32,
    pub enabled_addons: BTreeSet<String>,
}

pub struct ConfigTemplateEngine;

impl ConfigTemplateEngine {
    pub fn render(
        spec: &ConfigTemplateSpec,
        context: &TemplateContext,
    ) -> Result<String, FusionError> {
        fs_safety::validate_relative_path(&spec.path)?;
        let rendered = substitute(&spec.template, spec.format, context)?;
        match spec.format {
            ConfigFormat::Json => {
                serde_json::from_str::<serde_json::Value>(&rendered).map_err(|error| {
                    invalid("Rendered config is not valid JSON.")
                        .with_detail("reason", error.to_string())
                })?;
            }
            ConfigFormat::Toml => {
                toml::from_str::<toml::Value>(&rendered).map_err(|error| {
                    invalid("Rendered config is not valid TOML.")
                        .with_detail("reason", error.to_string())
                })?;
            }
            ConfigFormat::Ini => validate_ini(&rendered)?,
        }
        Ok(rendered)
    }

    /// Render everything before publishing. Replacements always unlink the previous
    /// file and publish a private file, never chmod or modify a projected CAS inode.
    /// `workspace_dir` is an owned, inactive workspace managed by the caller.
    pub fn render_to_workspace(
        templates: &[ConfigTemplateSpec],
        context: &TemplateContext,
    ) -> Result<Vec<String>, FusionError> {
        let _workspace = fs_safety::guard(&context.workspace_dir)?;
        let mut paths = BTreeSet::new();
        let mut rendered = Vec::new();
        for template in templates {
            if !paths.insert(template.path.to_ascii_lowercase()) {
                return Err(invalid("Config output paths must be unique."));
            }
            let relative = fs_safety::validate_relative_path(&template.path)?;
            rendered.push((relative, Self::render(template, context)?));
        }
        let mut written = Vec::new();
        for (relative, contents) in rendered {
            let output = context.workspace_dir.join(&relative);
            let _parent = fs_safety::ensure_directory(
                output
                    .parent()
                    .ok_or_else(|| invalid("Missing output parent."))?,
            )?;
            // atomic_write stages a private create_new file and replaces the old link safely.
            fs_safety::atomic_write(&output, contents.as_bytes())?;
            written.push(relative.to_string_lossy().replace('\\', "/"));
        }
        Ok(written)
    }
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidTemplate, message)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Double,
    Single,
}

fn substitute(
    template: &str,
    format: ConfigFormat,
    context: &TemplateContext,
) -> Result<String, FusionError> {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut comment = false;
    while !rest.is_empty() {
        if rest.starts_with("{{") {
            let end = rest
                .find("}}")
                .ok_or_else(|| invalid("Unclosed template variable."))?;
            let variable = &rest[2..end];
            if comment || quote == Quote::Single {
                return Err(invalid(
                    "Variables cannot appear in comments or literal strings.",
                ));
            }
            if escaped {
                return Err(invalid(
                    "A variable cannot follow an incomplete string escape.",
                ));
            }
            let (value, is_string) = variable_value(variable, context)?;
            match (format, quote) {
                (_, Quote::Double) => {
                    let encoded = serde_json::to_string(&value)
                        .map_err(|_| invalid("String encoding failed."))?;
                    output.push_str(&encoded[1..encoded.len() - 1]);
                }
                (ConfigFormat::Ini, Quote::None) => output.push_str(&value),
                (_, Quote::None) if !is_string => output.push_str(&value),
                _ => {
                    return Err(invalid(
                        "String variables must appear inside double-quoted strings.",
                    ))
                }
            }
            rest = &rest[end + 2..];
            continue;
        }
        if rest.starts_with("}}") {
            return Err(invalid("Unmatched template delimiter."));
        }
        // Templates deliberately use TOML basic strings. Reject multiline string
        // delimiters so their different escape rules cannot invalidate substitution.
        if format == ConfigFormat::Toml && (rest.starts_with("\"\"\"") || rest.starts_with("'''")) {
            return Err(invalid(
                "TOML templates use single-line basic or literal strings.",
            ));
        }
        let character = rest.chars().next().expect("nonempty template remainder");
        output.push(character);
        rest = &rest[character.len_utf8()..];
        if character == '\n' {
            comment = false;
        }
        if comment {
            continue;
        }
        match quote {
            Quote::Double => {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    quote = Quote::None;
                }
            }
            Quote::Single if character == '\'' => quote = Quote::None,
            Quote::Single => {}
            Quote::None => match character {
                '"' => quote = Quote::Double,
                '\'' if format != ConfigFormat::Json => quote = Quote::Single,
                '#' | ';' if format == ConfigFormat::Ini => comment = true,
                '#' if format == ConfigFormat::Toml => comment = true,
                _ => {}
            },
        }
    }
    if quote != Quote::None {
        return Err(invalid("Unclosed config string."));
    }
    Ok(output)
}

fn variable_value(
    variable: &str,
    context: &TemplateContext,
) -> Result<(String, bool), FusionError> {
    let string = match variable {
        "WORKSPACE_DIR" => Some(&context.workspace_dir),
        "APPDATA" => Some(&context.appdata_dir),
        "LOCALAPPDATA" => Some(&context.local_appdata_dir),
        _ => None,
    };
    if let Some(path) = string {
        let value = path
            .to_str()
            .ok_or_else(|| invalid("Template paths must be valid Unicode."))?;
        if value.chars().any(char::is_control) {
            return Err(invalid("Template paths cannot contain control characters."));
        }
        return Ok((value.to_owned(), true));
    }
    let value = match variable {
        "DISPLAY_WIDTH" => context.display_width.to_string(),
        "DISPLAY_HEIGHT" => context.display_height.to_string(),
        "REFRESH_RATE" => context.refresh_rate.to_string(),
        _ => {
            let id = variable.strip_prefix("ADDON_ENABLED:").ok_or_else(|| {
                invalid("Unknown template variable.").with_detail("variable", variable)
            })?;
            if id.is_empty()
                || id.len() > 128
                || matches!(id, "." | "..")
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err(invalid("Invalid addon identifier in template."));
            }
            context.enabled_addons.contains(id).to_string()
        }
    };
    Ok((value, false))
}

fn validate_ini(contents: &str) -> Result<(), FusionError> {
    let mut section = String::new();
    let mut keys = BTreeSet::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if line.chars().any(|c| c.is_control() && c != '\t') {
            return Err(invalid("INI config contains control characters."));
        }
        if line.starts_with('[') {
            if !line.ends_with(']')
                || line.len() < 3
                || line[1..line.len() - 1].contains(['[', ']'])
            {
                return Err(invalid("Invalid INI section."));
            }
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
        } else {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| invalid("INI entries must use key=value."))?;
            let key = key.trim();
            if key.is_empty()
                || key.contains(['[', ']', '"', '\''])
                || !keys.insert((section.clone(), key.to_ascii_lowercase()))
            {
                return Err(invalid("Invalid or duplicate INI key."));
            }
            let value = value.trim();
            if value.starts_with('"') && serde_json::from_str::<String>(value).is_err() {
                return Err(invalid("Invalid INI double-quoted string."));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn context(root: PathBuf) -> TemplateContext {
        TemplateContext {
            workspace_dir: root,
            appdata_dir: PathBuf::from(r"C:\Users\Example\AppData\Roaming"),
            local_appdata_dir: PathBuf::from(r"C:\Users\Example\AppData\Local"),
            display_width: 2560,
            display_height: 1440,
            refresh_rate: 120,
            enabled_addons: BTreeSet::from(["coop".into()]),
        }
    }

    fn spec(format: ConfigFormat, template: &str) -> ConfigTemplateSpec {
        ConfigTemplateSpec {
            id: "settings".into(),
            path: "settings/config".into(),
            format,
            template: template.into(),
        }
    }

    #[test]
    fn json_and_toml_preserve_windows_paths_and_types() {
        let ctx = context(PathBuf::from(r"C:\Library\Work space"));
        let json = ConfigTemplateEngine::render(&spec(ConfigFormat::Json, r#"{"path":"{{APPDATA}}/emu", "width":{{DISPLAY_WIDTH}}, "height":{{DISPLAY_HEIGHT}}, "rate":{{REFRESH_RATE}}, "coop":{{ADDON_ENABLED:coop}}, "other":{{ADDON_ENABLED:other}}}"#), &ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["path"], r"C:\Users\Example\AppData\Roaming/emu");
        assert_eq!(parsed["width"], 2560);
        assert_eq!(parsed["height"], 1440);
        assert_eq!(parsed["rate"], 120);
        assert_eq!(parsed["coop"], true);
        assert_eq!(parsed["other"], false);
        let text = ConfigTemplateEngine::render(
            &spec(
                ConfigFormat::Toml,
                "path = \"{{WORKSPACE_DIR}}/saves\"\ncoop = {{ADDON_ENABLED:coop}}\n",
            ),
            &ctx,
        )
        .unwrap();
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(
            parsed["path"].as_str(),
            Some(r"C:\Library\Work space/saves")
        );
        assert_eq!(parsed["coop"].as_bool(), Some(true));
    }

    #[test]
    fn ini_values_and_bad_templates() {
        let ctx = context(PathBuf::from(r"C:\Library"));
        let text = ConfigTemplateEngine::render(
            &spec(
                ConfigFormat::Ini,
                "[Display]\nwidth={{DISPLAY_WIDTH}}\npath={{LOCALAPPDATA}}/emu\n",
            ),
            &ctx,
        )
        .unwrap();
        assert!(text.contains(r"path=C:\Users\Example\AppData\Local/emu"));
        for template in [
            "x={{UNKNOWN}}",
            "x={{APPDATA",
            "x=}}",
            "x={{ADDON_ENABLED:../bad}}",
            "# {{APPDATA}}",
        ] {
            assert!(
                ConfigTemplateEngine::render(&spec(ConfigFormat::Ini, template), &ctx).is_err()
            );
        }
        assert!(ConfigTemplateEngine::render(
            &spec(ConfigFormat::Json, r#"{"path":{{APPDATA}}}"#),
            &ctx
        )
        .is_err());
        assert!(
            ConfigTemplateEngine::render(&spec(ConfigFormat::Json, r#"{"a":1,}"#), &ctx).is_err()
        );
        assert!(ConfigTemplateEngine::render(&spec(ConfigFormat::Toml, "a = ["), &ctx).is_err());
        assert!(
            ConfigTemplateEngine::render(&spec(ConfigFormat::Ini, "broken entry"), &ctx).is_err()
        );
    }

    #[test]
    fn rendered_output_detaches_readonly_hardlink_and_rejects_traversal() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let cas = temporary.path().join("cas");
        fs::write(&cas, b"original CAS bytes").unwrap();
        let mut permissions = fs::metadata(&cas).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&cas, permissions).unwrap();
        fs::hard_link(&cas, workspace.join("config.json")).unwrap();
        let ctx = context(workspace.clone());
        let mut config = spec(ConfigFormat::Json, r#"{"width":{{DISPLAY_WIDTH}}}"#);
        config.path = "config.json".into();
        ConfigTemplateEngine::render_to_workspace(&[config.clone()], &ctx).unwrap();
        assert_eq!(fs::read(&cas).unwrap(), b"original CAS bytes");
        assert!(fs::metadata(&cas).unwrap().permissions().readonly());
        assert!(!fs::metadata(workspace.join("config.json"))
            .unwrap()
            .permissions()
            .readonly());
        fs::write(workspace.join("config.json"), b"private edit").unwrap();
        assert_eq!(fs::read(&cas).unwrap(), b"original CAS bytes");
        config.path = "../outside.json".into();
        assert!(ConfigTemplateEngine::render_to_workspace(&[config], &ctx).is_err());
        fs_safety::unlink_file(&cas).unwrap();
    }
}
