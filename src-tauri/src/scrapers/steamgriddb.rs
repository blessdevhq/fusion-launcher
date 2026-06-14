use std::collections::BTreeMap;

use serde_json::Value;

use crate::schema::{GameMetadata, ScrapedGamePayload};

pub const PROVIDER: &str = "steamgriddb";
pub const DAILY_REQUEST_LIMIT: u32 = 1_000;

const BASE_URL: &str = "https://www.steamgriddb.com/api/v2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeySource {
    User,
    BuiltIn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedApiKey {
    pub key: String,
    pub source: ApiKeySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamGridDbGame {
    pub id: u64,
    pub name: String,
}

pub struct SteamGridDbClient {
    client: reqwest::Client,
    api_key: String,
}

impl SteamGridDbClient {
    pub fn new(api_key: String) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent("Fusion Launcher/0.1")
            .build()
            .map_err(|error| format!("Failed to create SteamGridDB client: {error}"))?;
        Ok(Self { client, api_key })
    }

    pub async fn autocomplete(&self, title: &str) -> Result<Vec<SteamGridDbGame>, String> {
        let value = self
            .get_json(&format!(
                "search/autocomplete/{}",
                encode_path_segment(title)
            ))
            .await?;
        Ok(data_items(&value)
            .into_iter()
            .filter_map(game_from_value)
            .collect())
    }

    pub async fn best_hero(&self, game_id: u64) -> Result<Option<String>, String> {
        let value = self.get_json(&format!("heroes/game/{game_id}")).await?;
        Ok(best_asset(&value, AssetKind::Hero))
    }

    pub async fn best_logo(&self, game_id: u64) -> Result<Option<String>, String> {
        let value = self.get_json(&format!("logos/game/{game_id}")).await?;
        Ok(best_asset(&value, AssetKind::Logo))
    }

    pub async fn best_grid(&self, game_id: u64) -> Result<Option<String>, String> {
        let value = self.get_json(&format!("grids/game/{game_id}")).await?;
        Ok(best_asset(&value, AssetKind::Grid))
    }

    async fn get_json(&self, path: &str) -> Result<Value, String> {
        let response = self
            .client
            .get(format!("{BASE_URL}/{path}"))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|error| format!("SteamGridDB request failed: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Value::Null);
        }
        if !response.status().is_success() {
            return Err(format!(
                "SteamGridDB returned HTTP {}",
                response.status().as_u16()
            ));
        }
        response
            .json::<Value>()
            .await
            .map_err(|error| format!("Failed to parse SteamGridDB response: {error}"))
    }
}

pub fn resolve_api_key(user_key: Option<&str>) -> Option<ResolvedApiKey> {
    resolve_api_key_values(
        user_key,
        first_non_empty(&[
            option_env!("FUSION_LAUNCHER_STEAMGRIDDB_KEY"),
            option_env!("RETROHYDRA_STEAMGRIDDB_KEY"),
        ]),
    )
}

pub fn resolve_api_key_values(
    user_key: Option<&str>,
    built_in_key: Option<&str>,
) -> Option<ResolvedApiKey> {
    user_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|key| ResolvedApiKey {
            key: key.to_string(),
            source: ApiKeySource::User,
        })
        .or_else(|| {
            built_in_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|key| ResolvedApiKey {
                    key: key.to_string(),
                    source: ApiKeySource::BuiltIn,
                })
        })
}

fn first_non_empty(candidates: &[Option<&'static str>]) -> Option<&'static str> {
    candidates
        .iter()
        .flatten()
        .copied()
        .find(|value| !value.trim().is_empty())
}

pub fn best_game_for_query(games: &[SteamGridDbGame], query: &str) -> Option<SteamGridDbGame> {
    let query = normalize_name(query);
    games
        .iter()
        .max_by_key(|game| {
            let name = normalize_name(&game.name);
            let mut score = 0;
            if name == query {
                score += 100;
            } else if name.contains(&query) || query.contains(&name) {
                score += 50;
            }
            score -= (name.len() as i32 - query.len() as i32).abs().min(30);
            score
        })
        .cloned()
}

pub fn payload_from_artwork(
    game: &SteamGridDbGame,
    hero: Option<String>,
    logo: Option<String>,
    grid: Option<String>,
) -> ScrapedGamePayload {
    let mut external_ids = BTreeMap::new();
    external_ids.insert(PROVIDER.to_string(), game.id.to_string());

    ScrapedGamePayload {
        provider: PROVIDER.to_string(),
        provider_game_id: Some(game.id.to_string()),
        title: Some(game.name.clone()),
        description: None,
        cover: grid,
        hero,
        logo,
        screenshots: Vec::new(),
        metadata: GameMetadata {
            release_year: None,
            developer: None,
            publisher: None,
            genres: Vec::new(),
            tags: vec!["artwork".to_string()],
            players: None,
            series: None,
            external_ids,
        },
    }
}

#[derive(Debug, Clone, Copy)]
enum AssetKind {
    Hero,
    Logo,
    Grid,
}

fn game_from_value(value: &Value) -> Option<SteamGridDbGame> {
    Some(SteamGridDbGame {
        id: value.get("id")?.as_u64()?,
        name: first_string(value, &["name", "title"])?,
    })
}

fn best_asset(value: &Value, kind: AssetKind) -> Option<String> {
    data_items(value)
        .into_iter()
        .filter_map(|item| {
            if bool_field(item, "nsfw").unwrap_or(false)
                || bool_field(item, "humor").unwrap_or(false)
            {
                return None;
            }
            let url = first_string(item, &["url", "thumb"])?;
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return None;
            }
            Some((asset_score(item, &url, kind), url))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, url)| url)
}

fn asset_score(value: &Value, url: &str, kind: AssetKind) -> i32 {
    let style = first_string(value, &["style"])
        .unwrap_or_default()
        .to_lowercase();
    let width = number_field(value, "width").unwrap_or(0.0);
    let height = number_field(value, "height").unwrap_or(0.0);
    let ratio = if height > 0.0 { width / height } else { 0.0 };
    let pixels = (width * height / 20_000.0).min(80.0) as i32;

    let mut score = pixels;
    if style.contains("official") {
        score += 100;
    } else if style.contains("alternate") {
        score += 70;
    } else if style.contains("material") {
        score += 45;
    } else if style.contains("blur") {
        score -= 30;
    }

    match kind {
        AssetKind::Hero => {
            if ratio >= 1.4 {
                score += 45;
            }
        }
        AssetKind::Logo => {
            if style.contains("transparent") || style.contains("white") {
                score += 45;
            }
            if url.to_lowercase().ends_with(".png") {
                score += 20;
            }
        }
        AssetKind::Grid => {
            if (0.55..=0.8).contains(&ratio) {
                score += 45;
            }
        }
    }

    score
}

fn data_items(value: &Value) -> Vec<&Value> {
    match value.get("data").unwrap_or(value) {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![value.get("data").unwrap_or(value)],
        _ => Vec::new(),
    }
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find_map(|item| match item {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .filter(|value| !value.trim().is_empty())
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
    match value.get(key)? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => Some(value.eq_ignore_ascii_case("true") || value == "1"),
        Value::Number(value) => Some(value.as_u64() == Some(1)),
        _ => None,
    }
}

fn number_field(value: &Value, key: &str) -> Option<f64> {
    match value.get(key)? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.parse::<f64>().ok(),
        _ => None,
    }
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.trim().as_bytes()).collect()
}

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_user_key_before_builtin_key() {
        let resolved = resolve_api_key_values(Some(" user-key "), Some("built-in")).unwrap();
        assert_eq!(resolved.key, "user-key");
        assert_eq!(resolved.source, ApiKeySource::User);
    }

    #[test]
    fn resolves_builtin_key_when_user_key_is_blank() {
        let resolved = resolve_api_key_values(Some(" "), Some(" built-in ")).unwrap();
        assert_eq!(resolved.key, "built-in");
        assert_eq!(resolved.source, ApiKeySource::BuiltIn);
    }

    #[test]
    fn resolves_no_key_when_all_sources_are_empty() {
        assert!(resolve_api_key_values(Some(" "), None).is_none());
    }

    #[test]
    fn parses_search_results_and_prefers_exact_title() {
        let games = data_items(&json!({
            "data": [
                { "id": 1, "name": "Metroid Prime Remastered" },
                { "id": 2, "name": "Metroid Prime" }
            ]
        }))
        .into_iter()
        .filter_map(game_from_value)
        .collect::<Vec<_>>();

        let game = best_game_for_query(&games, "Metroid Prime").unwrap();
        assert_eq!(game.id, 2);
    }

    #[test]
    fn filters_nsfw_and_picks_best_grid_asset() {
        let value = json!({
            "data": [
                { "url": "https://cdn.example/bad.jpg", "nsfw": true, "humor": false, "style": "official", "width": 600, "height": 900 },
                { "url": "https://cdn.example/good.jpg", "nsfw": false, "humor": false, "style": "official", "width": 600, "height": 900 }
            ]
        });

        assert_eq!(
            best_asset(&value, AssetKind::Grid).as_deref(),
            Some("https://cdn.example/good.jpg")
        );
    }
}
