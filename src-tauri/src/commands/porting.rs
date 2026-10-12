use crate::schema::{GameArtwork, GameLaunchConfig, GameMetadata};

const USER_REPO_ID: &str = "user-library";
const USER_REPO_URL: &str = "custom://user-library";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameFileInspection {
    pub path: String,
    pub file_name: String,
    pub suggested_title: String,
    pub platform: String,
    pub setup_profile_id: String,
    pub expected_extensions: Vec<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub crc32: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomGameInput {
    pub id: Option<String>,
    pub title: String,
    pub platform: String,
    pub description: Option<String>,
    pub cover_image_url: Option<String>,
    pub hero_image_url: Option<String>,
    pub release_year: Option<u32>,
    pub developer: Option<String>,
    pub genres: Option<Vec<String>>,
    pub content_mode: Option<String>,
    pub download_url: Option<String>,
    pub magnet_uri: Option<String>,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
    pub setup_profile_id: Option<String>,
    pub expected_extensions: Option<Vec<String>>,
    pub author_name: Option<String>,
    pub author_url: Option<String>,
    pub patch_version: Option<String>,
    pub notes: Option<String>,
    pub enable_60fps: Option<bool>,
    pub shader_cache_url: Option<String>,
    pub import_source_path: Option<String>,
}

pub fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "custom-game".to_string()
    } else {
        trimmed
    }
}

pub fn clean_game_title(raw_name: &str) -> String {
    let without_ext = Path::new(raw_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(raw_name);

    let mut result = String::new();
    let mut in_paren = 0;
    let mut in_bracket = 0;

    for c in without_ext.chars() {
        match c {
            '(' => in_paren += 1,
            ')' => {
                if in_paren > 0 {
                    in_paren -= 1;
                }
            }
            '[' => in_bracket += 1,
            ']' => {
                if in_bracket > 0 {
                    in_bracket -= 1;
                }
            }
            _ => {
                if in_paren == 0 && in_bracket == 0 {
                    if c == '_' {
                        result.push(' ');
                    } else {
                        result.push(c);
                    }
                }
            }
        }
    }

    // Collapse spaces and dashes
    let words: Vec<&str> = result.split_whitespace().collect();
    let joined = words.join(" ");
    let cleaned = joined.trim_matches(|c: char| c == '-' || c == ' ').trim();

    if cleaned.is_empty() {
        without_ext.trim().to_string()
    } else {
        cleaned.to_string()
    }
}

pub fn detect_platform_from_path(path: &Path) -> (&'static str, &'static str, Vec<String>) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "nsp" | "xci" | "nca" => (
            "switch",
            "switch-manual",
            vec![".nsp".into(), ".xci".into(), ".nca".into()],
        ),
        "iso" | "chd" => (
            "ps2",
            "ps2-pcsx2",
            vec![".iso".into(), ".chd".into(), ".bin".into(), ".img".into()],
        ),
        "cso" => (
            "psp",
            "psp-ppsspp",
            vec![".iso".into(), ".cso".into(), ".pbp".into()],
        ),
        "z64" | "n64" | "v64" => (
            "n64",
            "n64-rmg",
            vec![".z64".into(), ".n64".into(), ".v64".into()],
        ),
        "sfc" | "smc" => (
            "snes",
            "snes-mesen",
            vec![".sfc".into(), ".smc".into()],
        ),
        "nes" => (
            "nes",
            "nes-mesen",
            vec![".nes".into()],
        ),
        "gba" => (
            "gba",
            "gba-mgba",
            vec![".gba".into()],
        ),
        "cue" | "pbp" => (
            "ps1",
            "ps1-manual",
            vec![
                ".cue".into(),
                ".bin".into(),
                ".iso".into(),
                ".pbp".into(),
                ".chd".into(),
            ],
        ),
        _ => ("ps2", "ps2-pcsx2", vec![format!(".{ext}")]),
    }
}

#[tauri::command]
pub fn inspect_game_file(path: String) -> Result<GameFileInspection, String> {
    let p = Path::new(&path);
    if !p.exists() || !p.is_file() {
        return Err(format!("File does not exist or is not a file: {path}"));
    }

    let file_name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("game")
        .to_string();

    let (platform, setup_profile_id, expected_extensions) = detect_platform_from_path(p);
    let suggested_title = clean_game_title(&file_name);

    let hashes = crate::rom_hasher::hash_rom(p, |_, _| {})
        .map_err(|e| format!("Failed to hash file: {e}"))?;

    Ok(GameFileInspection {
        path,
        file_name,
        suggested_title,
        platform: platform.to_string(),
        setup_profile_id: setup_profile_id.to_string(),
        expected_extensions,
        size_bytes: hashes.size,
        sha256: hashes.sha256,
        crc32: hashes.crc32,
    })
}

fn create_default_user_repo() -> RepositorySchema {
    RepositorySchema {
        metadata: RepositoryMetadata {
            id: USER_REPO_ID.to_string(),
            name: "Community & User Library".to_string(),
            version: "1.0.0".to_string(),
            schema_version: 3,
            maintainer: Some("Fusion Community".to_string()),
            homepage_url: None,
            license: None,
            trust_level: Some("community".to_string()),
            content_hash: None,
            updated_at: Some(chrono::Utc::now().to_rfc3339()),
        },
        system_files: Vec::new(),
        catalog: Vec::new(),
    }
}

#[tauri::command]
pub async fn add_custom_game(
    input: CustomGameInput,
    state: State<'_, AppState>,
) -> Result<CatalogGameView, String> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err("Game title cannot be empty.".to_string());
    }

    let game_id = input
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(slugify)
        .unwrap_or_else(|| slugify(&title));

    let platform = input.platform.trim().to_ascii_lowercase();
    let setup_profile_id = input
        .setup_profile_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| match platform.as_str() {
            "switch" => "switch-manual".to_string(),
            "ps2" => "ps2-pcsx2".to_string(),
            "ps1" => "ps1-manual".to_string(),
            "psp" => "psp-ppsspp".to_string(),
            "n64" => "n64-rmg".to_string(),
            "snes" => "snes-mesen".to_string(),
            "nes" => "nes-mesen".to_string(),
            "gba" => "gba-mgba".to_string(),
            _ => "ps2-pcsx2".to_string(),
        });

    let expected_extensions = input
        .expected_extensions
        .clone()
        .filter(|exts| !exts.is_empty())
        .unwrap_or_else(|| match platform.as_str() {
            "switch" => vec![".nsp".into(), ".xci".into(), ".nca".into()],
            "ps2" => vec![".iso".into(), ".chd".into(), ".bin".into(), ".img".into()],
            "ps1" => vec![".cue".into(), ".bin".into(), ".iso".into(), ".pbp".into(), ".chd".into()],
            "psp" => vec![".iso".into(), ".cso".into(), ".pbp".into()],
            "n64" => vec![".z64".into(), ".n64".into(), ".v64".into()],
            "snes" => vec![".sfc".into(), ".smc".into()],
            "nes" => vec![".nes".into()],
            "gba" => vec![".gba".into()],
            _ => vec![".iso".into()],
        });

    let mut downloads = Vec::new();
    let content_mode: Option<String>;

    if let Some(magnet) = input.magnet_uri.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        downloads.push(SourceUri::Magnet {
            uri: magnet.to_string(),
            info_hash: None,
            size_bytes: input.size_bytes,
        });
        content_mode = Some("downloadable".to_string());
    } else if let Some(http_url) = input.download_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        downloads.push(SourceUri::Http {
            url: http_url.to_string(),
            sha256: input.sha256.clone().unwrap_or_default(),
            size_bytes: input.size_bytes,
        });
        content_mode = Some("downloadable".to_string());
    } else {
        downloads.push(SourceUri::UserProvided {
            instructions: Some("Import your own legally dumped game file to play.".to_string()),
            sha256: input.sha256.clone(),
            size_bytes: input.size_bytes,
        });
        content_mode = Some("user_provided".to_string());
    }

    let mut tags = Vec::new();
    let mut external_ids = std::collections::BTreeMap::new();

    if let Some(author) = input.author_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(format!("author:{author}"));
        external_ids.insert("author".to_string(), author.to_string());
    }
    if let Some(author_url) = input.author_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(format!("authorUrl:{author_url}"));
        external_ids.insert("authorUrl".to_string(), author_url.to_string());
    }
    if let Some(patch) = input.patch_version.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(format!("patch:{patch}"));
        external_ids.insert("patch".to_string(), patch.to_string());
    }
    if let Some(notes) = input.notes.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(format!("notes:{notes}"));
        external_ids.insert("notes".to_string(), notes.to_string());
    }

    let game_metadata = GameMetadata {
        release_year: input.release_year.map(|y| y as u16),
        developer: input.developer.clone(),
        publisher: None,
        genres: input.genres.clone().unwrap_or_default(),
        tags,
        players: None,
        series: None,
        external_ids,
    };

    let artwork = GameArtwork {
        cover: input.cover_image_url.clone(),
        hero: input.hero_image_url.clone(),
        logo: None,
        screenshots: Vec::new(),
    };

    let launch_config = if input.enable_60fps == Some(true) || input.shader_cache_url.is_some() {
        Some(GameLaunchConfig {
            args_template: None,
            preferred_file: None,
            inject_mods: if input.enable_60fps == Some(true) {
                vec!["60fps".to_string()]
            } else {
                Vec::new()
            },
            shader_cache_url: input.shader_cache_url.clone(),
        })
    } else {
        None
    };

    let new_game = RepositoryGame {
        id: game_id.clone(),
        platform: platform.clone(),
        title: title.clone(),
        description: input.description.clone(),
        cover_image_url: input.cover_image_url.clone(),
        trailer_url: None,
        artwork: Some(artwork),
        metadata: Some(game_metadata),
        content_mode,
        setup_profile_id: Some(setup_profile_id),
        downloads,
        expected_extensions,
        required_system_file_ids: Vec::new(),
        launch: launch_config,
    };

    let storage_id = crate::security::global_id(USER_REPO_ID, &game_id);
    let target_download_root = download_root(&state)?;

    {
        let mut store = lock_store(&state)?;
        let existing_raw = store.get_repository_raw_json(USER_REPO_ID)?;

        let mut repo: RepositorySchema = if let Some(raw) = existing_raw {
            serde_json::from_str(&raw).unwrap_or_else(|_| create_default_user_repo())
        } else {
            create_default_user_repo()
        };

        repo.catalog.retain(|g| g.id != game_id);
        repo.catalog.push(new_game);
        repo.metadata.updated_at = Some(chrono::Utc::now().to_rfc3339());

        store.store_repository(USER_REPO_URL, &repo)?;

        // If a local file path was provided for import, import it into the store right away
        if let Some(import_path) = input.import_source_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let p = Path::new(import_path);
            if p.exists() && p.is_file() {
                let _ = import_game_file_into_store(
                    &store,
                    &target_download_root,
                    &storage_id,
                    p,
                );
            }
        }
    }

    let store = lock_store(&state)?;
    let game_view = store
        .get_game(&storage_id)?
        .ok_or_else(|| format!("Game '{storage_id}' was stored but could not be read back."))?;

    Ok(game_view)
}

#[tauri::command]
pub fn export_game_package(
    input: CustomGameInput,
    export_path: String,
) -> Result<String, String> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err("Game title cannot be empty.".to_string());
    }

    let target_path = Path::new(&export_path);
    if let Some(parent) = target_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directories: {e}"))?;
        }
    }

    let serialized = serde_json::to_string_pretty(&input)
        .map_err(|e| format!("Failed to serialize game manifest: {e}"))?;

    std::fs::write(target_path, serialized)
        .map_err(|e| format!("Failed to write export file: {e}"))?;

    Ok(export_path)
}

#[cfg(test)]
mod porting_tests {
    use super::*;

    #[test]
    fn cleans_game_title_from_messy_dump_names() {
        assert_eq!(
            clean_game_title("Super Mario 64 (USA) [!].z64"),
            "Super Mario 64"
        );
        assert_eq!(
            clean_game_title("The_Legend_of_Zelda_-_Breath_of_the_Wild_(USA)_(v1.6.0).nsp"),
            "The Legend of Zelda - Breath of the Wild"
        );
        assert_eq!(
            clean_game_title("Silent Hill 2 (Director's Cut) (Europe) (En,Fr,De).iso"),
            "Silent Hill 2"
        );
        assert_eq!(
            clean_game_title("Chrono_Trigger_[T-Ru_v1.0].sfc"),
            "Chrono Trigger"
        );
    }

    #[test]
    fn detects_platforms_accurately() {
        assert_eq!(
            detect_platform_from_path(Path::new("game.nsp")).0,
            "switch"
        );
        assert_eq!(
            detect_platform_from_path(Path::new("game.iso")).0,
            "ps2"
        );
        assert_eq!(
            detect_platform_from_path(Path::new("game.z64")).0,
            "n64"
        );
        assert_eq!(
            detect_platform_from_path(Path::new("game.sfc")).0,
            "snes"
        );
        assert_eq!(
            detect_platform_from_path(Path::new("game.gba")).0,
            "gba"
        );
        assert_eq!(
            detect_platform_from_path(Path::new("game.cso")).0,
            "psp"
        );
    }

    #[test]
    fn slugifies_properly() {
        assert_eq!(slugify("Super Mario 64"), "super-mario-64");
        assert_eq!(slugify("Zelda: Breath of the Wild!"), "zelda-breath-of-the-wild");
        assert_eq!(slugify(""), "custom-game");
    }
}
