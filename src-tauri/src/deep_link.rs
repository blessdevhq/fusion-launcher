use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

const MAX_PARAM_LEN: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "params",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DeepLinkAction {
    AddSource { url: String },
    Install { manifest: String },
    Play { game_id: String },
}

fn https_url(raw: &str) -> Result<String, String> {
    if raw.len() > MAX_PARAM_LEN {
        return Err("deep_link_param_too_long".into());
    }
    let url = Url::parse(raw.trim()).map_err(|_| "deep_link_bad_url".to_string())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("deep_link_https_required".into());
    }
    Ok(url.to_string())
}

fn safe_id(raw: &str) -> Result<String, String> {
    let ok = !raw.is_empty()
        && raw.len() <= 128
        && raw != "."
        && raw != ".."
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(raw.to_string())
    } else {
        Err("deep_link_bad_id".into())
    }
}

fn param<'a>(q: &'a HashMap<String, String>, keys: &[&str]) -> Result<&'a str, String> {
    keys.iter()
        .find_map(|k| q.get(*k))
        .map(String::as_str)
        .ok_or_else(|| "deep_link_missing_param".to_string())
}

pub fn parse_deep_link(raw: &str) -> Result<DeepLinkAction, String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with("fusion://") && !trimmed.starts_with("fusion:") {
        return Err("deep_link_scheme".into());
    }
    let url = Url::parse(trimmed).map_err(|_| "deep_link_invalid".to_string())?;
    if url.scheme() != "fusion" {
        return Err("deep_link_scheme".into());
    }
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    match url.host_str().unwrap_or("").to_ascii_lowercase().as_str() {
        "source" => Ok(DeepLinkAction::AddSource {
            url: https_url(param(&q, &["url"])?)?,
        }),
        "install" => Ok(DeepLinkAction::Install {
            manifest: https_url(param(&q, &["manifest", "url"])?)?,
        }),
        "play" => Ok(DeepLinkAction::Play {
            game_id: safe_id(param(&q, &["game", "id"])?)?,
        }),
        _ => Err("deep_link_unsupported_action".into()),
    }
}

#[cfg(windows)]
pub fn register_windows_protocol() -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current executable path: {e}"))?;
    let exe_path = current_exe.to_string_lossy().to_string();

    let run_reg = |args: &[&str]| -> Result<(), String> {
        let output = std::process::Command::new("reg")
            .args(args)
            .output()
            .map_err(|e| format!("Failed to execute reg: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Registry command failed for {:?}: {stderr}", args));
        }
        Ok(())
    };

    run_reg(&[
        "add",
        r"HKCU\Software\Classes\fusion",
        "/ve",
        "/d",
        "URL:Fusion Protocol",
        "/f",
    ])?;

    run_reg(&[
        "add",
        r"HKCU\Software\Classes\fusion",
        "/v",
        "URL Protocol",
        "/d",
        "",
        "/f",
    ])?;

    let command_value = format!("\"{}\" \"%1\"", exe_path);
    run_reg(&[
        "add",
        r"HKCU\Software\Classes\fusion\shell\open\command",
        "/ve",
        "/d",
        &command_value,
        "/f",
    ])?;

    Ok(())
}

#[cfg(not(windows))]
pub fn register_windows_protocol() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dangerous_links() {
        for bad in [
            "fusion://import?path=%5C%5Cevil%5Cshare%5Ca.nsp",
            "fusion://game/add?data=eyJ9",
            "fusion://source/add",
            "fusion://source/add?url=http%3A%2F%2Fexample.com%2Fr.json",
            "fusion://source/add?url=file%3A%2F%2F%2FC%3A%2Fx.json",
            "fusion://source/add?url=https%3A%2F%2Fu%3Ap%40example.com%2Fr.json",
            "fusion://install?manifest=javascript%3Aalert(1)",
            "fusion://play?game=..%5C..%5Cx",
            "fusion://play?game=",
            "fusion://play?game=.",
            "fusion://play?game=..",
        ] {
            assert!(parse_deep_link(bad).is_err(), "Should reject: {bad}");
        }
    }

    #[test]
    fn accepts_the_three_supported_actions() {
        assert_eq!(
            parse_deep_link("fusion://play?game=abc-1").unwrap(),
            DeepLinkAction::Play {
                game_id: "abc-1".into()
            }
        );
        assert_eq!(
            parse_deep_link("fusion://source/add?url=https%3A%2F%2Fexample.com%2Fr.json").unwrap(),
            DeepLinkAction::AddSource {
                url: "https://example.com/r.json".into()
            }
        );
        assert_eq!(
            parse_deep_link("fusion://install?manifest=https%3A%2F%2Fexample.com%2Fg.json")
                .unwrap(),
            DeepLinkAction::Install {
                manifest: "https://example.com/g.json".into()
            }
        );
    }

    #[test]
    fn rejects_invalid_schemes() {
        assert!(parse_deep_link("https://example.com").is_err());
        assert!(parse_deep_link("steam://run/1234").is_err());
    }

    #[test]
    fn play_params_use_camel_case() {
        let action = DeepLinkAction::Play {
            game_id: "smw-1".into(),
        };
        let serialized = serde_json::to_string(&action).unwrap();
        assert_eq!(
            serialized,
            r#"{"action":"play","params":{"gameId":"smw-1"}}"#
        );
    }
}
