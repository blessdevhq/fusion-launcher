use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "params", rename_all = "camelCase")]
pub enum DeepLinkAction {
    AddSource {
        url: String,
        name: Option<String>,
    },
    AddGame {
        data: String,
        title: Option<String>,
    },
    Install {
        manifest: String,
        title: Option<String>,
    },
    Play {
        game_id: String,
    },
    Import {
        path: String,
    },
    Unknown {
        uri: String,
    },
}

pub fn decode_base64_data_if_needed(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        return trimmed.to_string();
    }

    for engine in [&URL_SAFE_NO_PAD, &URL_SAFE, &STANDARD] {
        if let Ok(bytes) = engine.decode(trimmed) {
            if let Ok(utf8) = String::from_utf8(bytes) {
                if utf8.trim_start().starts_with('{') || utf8.trim_start().starts_with('[') {
                    return utf8;
                }
            }
        }
    }

    trimmed.to_string()
}

pub fn parse_deep_link(raw_uri: &str) -> Result<DeepLinkAction, String> {
    let trimmed = raw_uri.trim();
    if !trimmed.starts_with("fusion://") && !trimmed.starts_with("fusion:") {
        return Err("URI must start with fusion://".to_string());
    }

    let url = Url::parse(trimmed).map_err(|e| format!("Invalid URL: {e}"))?;
    if url.scheme() != "fusion" {
        return Err("Scheme is not fusion".to_string());
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = url.path().trim_start_matches('/').to_ascii_lowercase();
    let query_map: HashMap<String, String> = url.query_pairs().into_owned().collect();

    if host == "game" || (host.is_empty() && path.starts_with("game")) {
        if let Some(data) = query_map.get("data").or_else(|| query_map.get("config")) {
            let decoded = decode_base64_data_if_needed(data);
            return Ok(DeepLinkAction::AddGame {
                data: decoded,
                title: query_map.get("title").cloned(),
            });
        }
    } else if host == "source" || (host.is_empty() && path.starts_with("source")) {
        if let Some(data) = query_map.get("data") {
            let decoded = decode_base64_data_if_needed(data);
            return Ok(DeepLinkAction::AddSource {
                url: decoded,
                name: query_map.get("name").cloned(),
            });
        }
        if let Some(source_url) = query_map.get("url") {
            return Ok(DeepLinkAction::AddSource {
                url: source_url.clone(),
                name: query_map.get("name").cloned(),
            });
        }
    } else if host == "install" || (host.is_empty() && path.starts_with("install")) {
        if let Some(data) = query_map.get("data") {
            let decoded = decode_base64_data_if_needed(data);
            return Ok(DeepLinkAction::Install {
                manifest: decoded,
                title: query_map.get("title").cloned(),
            });
        }
        if let Some(manifest) = query_map.get("manifest").or_else(|| query_map.get("url")) {
            return Ok(DeepLinkAction::Install {
                manifest: manifest.clone(),
                title: query_map.get("title").cloned(),
            });
        }
    } else if host == "play" || (host.is_empty() && path.starts_with("play")) {
        if let Some(game_id) = query_map.get("game").or_else(|| query_map.get("id")) {
            return Ok(DeepLinkAction::Play {
                game_id: game_id.clone(),
            });
        }
    } else if host == "import" || (host.is_empty() && path.starts_with("import")) {
        if let Some(file_path) = query_map.get("path").or_else(|| query_map.get("file")) {
            return Ok(DeepLinkAction::Import {
                path: file_path.clone(),
            });
        }
    }

    Ok(DeepLinkAction::Unknown {
        uri: trimmed.to_string(),
    })
}

#[cfg(windows)]
pub fn register_windows_protocol() -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current executable path: {e}"))?;
    let exe_path = current_exe.to_string_lossy().to_string();

    let _ = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Classes\fusion",
            "/ve",
            "/d",
            "URL:Fusion Protocol",
            "/f",
        ])
        .output();

    let _ = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Classes\fusion",
            "/v",
            "URL Protocol",
            "/d",
            "",
            "/f",
        ])
        .output();

    let command_value = format!("\"{}\" \"%1\"", exe_path);
    let _ = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Classes\fusion\shell\open\command",
            "/ve",
            "/d",
            &command_value,
            "/f",
        ])
        .output();

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
    fn parses_add_source_deep_link() {
        let uri = "fusion://source/add?url=https%3A%2F%2Fexample.com%2Frepo.json&name=My%20Repo";
        let action = parse_deep_link(uri).unwrap();

        assert_eq!(
            action,
            DeepLinkAction::AddSource {
                url: "https://example.com/repo.json".to_string(),
                name: Some("My Repo".to_string()),
            }
        );
    }

    #[test]
    fn parses_install_manifest_deep_link() {
        let uri = "fusion://install?manifest=https%3A%2F%2Fexample.com%2Fzelda.json&title=Zelda";
        let action = parse_deep_link(uri).unwrap();

        assert_eq!(
            action,
            DeepLinkAction::Install {
                manifest: "https://example.com/zelda.json".to_string(),
                title: Some("Zelda".to_string()),
            }
        );
    }

    #[test]
    fn parses_play_game_deep_link() {
        let uri = "fusion://play?game=0100F2C0115B6000";
        let action = parse_deep_link(uri).unwrap();

        assert_eq!(
            action,
            DeepLinkAction::Play {
                game_id: "0100F2C0115B6000".to_string(),
            }
        );
    }

    #[test]
    fn parses_import_deep_link() {
        let uri = "fusion://import?path=C%3A%5CGames%5CZelda.nsp";
        let action = parse_deep_link(uri).unwrap();

        assert_eq!(
            action,
            DeepLinkAction::Import {
                path: "C:\\Games\\Zelda.nsp".to_string(),
            }
        );
    }

    #[test]
    fn parses_add_game_base64_deep_link() {
        let json = r#"{"title":"Chrono Trigger","platform":"snes"}"#;
        let encoded = URL_SAFE_NO_PAD.encode(json);
        let uri = format!("fusion://game/add?data={encoded}&title=Chrono");
        let action = parse_deep_link(&uri).unwrap();

        assert_eq!(
            action,
            DeepLinkAction::AddGame {
                data: json.to_string(),
                title: Some("Chrono".to_string()),
            }
        );
    }

    #[test]
    fn parses_add_source_base64_deep_link() {
        let json = r#"{"name":"Community Tap","catalog":[]}"#;
        let encoded = URL_SAFE_NO_PAD.encode(json);
        let uri = format!("fusion://source/add?data={encoded}&name=Tap");
        let action = parse_deep_link(&uri).unwrap();

        assert_eq!(
            action,
            DeepLinkAction::AddSource {
                url: json.to_string(),
                name: Some("Tap".to_string()),
            }
        );
    }

    #[test]
    fn rejects_invalid_schemes() {
        assert!(parse_deep_link("https://example.com").is_err());
        assert!(parse_deep_link("steam://run/1234").is_err());
    }
}
