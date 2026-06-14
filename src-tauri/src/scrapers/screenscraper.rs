use std::collections::BTreeMap;

use serde_json::Value;

use crate::schema::{GameMetadata, ScrapeCandidate, ScrapedGamePayload};

pub const PROVIDER: &str = "screenscraper";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub ssid: String,
    pub sspassword: String,
}

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub language: String,
    pub region: String,
}

pub struct ScreenScraperClient {
    client: reqwest::Client,
    credentials: Credentials,
    options: RequestOptions,
}

impl ScreenScraperClient {
    pub fn new(credentials: Credentials, options: RequestOptions) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent("Fusion Launcher/0.1")
            .build()
            .map_err(|error| format!("Failed to create ScreenScraper client: {error}"))?;
        Ok(Self {
            client,
            credentials,
            options,
        })
    }

    pub async fn by_hash(
        &self,
        system_id: u32,
        crc32: &str,
    ) -> Result<Option<ScrapedGamePayload>, String> {
        let Some(value) = self
            .get_json(
                "jeuInfos.php",
                &[
                    ("systemeid", system_id.to_string()),
                    ("crc", crc32.to_lowercase()),
                ],
            )
            .await?
        else {
            return Ok(None);
        };

        Ok(game_node(&value).and_then(|game| payload_from_game(game, &self.options, "hash")))
    }

    pub async fn by_id(
        &self,
        provider_game_id: &str,
    ) -> Result<Option<ScrapedGamePayload>, String> {
        let Some(value) = self
            .get_json(
                "jeuInfos.php",
                &[("gameid", provider_game_id.trim().to_string())],
            )
            .await?
        else {
            return Ok(None);
        };

        Ok(game_node(&value).and_then(|game| payload_from_game(game, &self.options, "name")))
    }

    pub async fn by_name(
        &self,
        system_id: u32,
        title: &str,
    ) -> Result<Vec<ScrapeCandidate>, String> {
        let Some(value) = self
            .get_json(
                "jeuRecherche.php",
                &[
                    ("systemeid", system_id.to_string()),
                    ("recherche", title.trim().to_string()),
                ],
            )
            .await?
        else {
            return Ok(Vec::new());
        };

        Ok(game_nodes(&value)
            .into_iter()
            .filter_map(|game| candidate_from_game(game, &self.options))
            .take(8)
            .collect())
    }

    async fn get_json(
        &self,
        endpoint: &str,
        params: &[(&str, String)],
    ) -> Result<Option<Value>, String> {
        let mut query = vec![
            ("output".to_string(), "json".to_string()),
            ("softname".to_string(), "Fusion Launcher".to_string()),
            ("ssid".to_string(), self.credentials.ssid.clone()),
            (
                "sspassword".to_string(),
                self.credentials.sspassword.clone(),
            ),
        ];
        if let Some((devid, devpassword)) = dev_credentials() {
            query.push(("devid".to_string(), devid));
            query.push(("devpassword".to_string(), devpassword));
        }
        query.extend(
            params
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone())),
        );

        let url = format!("https://www.screenscraper.fr/api2/{endpoint}");
        let response = self
            .client
            .get(url)
            .query(&query)
            .send()
            .await
            .map_err(|error| format!("ScreenScraper request failed: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(format!(
                "ScreenScraper returned HTTP {}",
                response.status().as_u16()
            ));
        }

        let value = response
            .json::<Value>()
            .await
            .map_err(|error| format!("Failed to parse ScreenScraper response: {error}"))?;
        if response_success_is_false(&value)
            && game_node(&value).is_none()
            && game_nodes(&value).is_empty()
        {
            if response_looks_like_miss(&value) {
                return Ok(None);
            }
            return Err(response_message(&value)
                .unwrap_or_else(|| "ScreenScraper rejected the request.".to_string()));
        }

        Ok(Some(value))
    }
}

/// Resolve the ScreenScraper developer credentials used to lift the anonymous
/// rate limits. A runtime env var wins (local dev override); otherwise the
/// value baked at compile time via `option_env!` is used (official builds).
/// Both id and password must be present and non-empty, or no devid is sent.
fn dev_credentials() -> Option<(String, String)> {
    let devid = resolve_dev_value(&[
        std::env::var("FUSION_LAUNCHER_SCREENSCRAPER_DEVID").ok(),
        std::env::var("RETROHYDRA_SCREENSCRAPER_DEVID").ok(),
        option_env!("FUSION_LAUNCHER_SCREENSCRAPER_DEVID").map(str::to_string),
        option_env!("RETROHYDRA_SCREENSCRAPER_DEVID").map(str::to_string),
    ])?;
    let devpassword = resolve_dev_value(&[
        std::env::var("FUSION_LAUNCHER_SCREENSCRAPER_DEVPASSWORD").ok(),
        std::env::var("RETROHYDRA_SCREENSCRAPER_DEVPASSWORD").ok(),
        option_env!("FUSION_LAUNCHER_SCREENSCRAPER_DEVPASSWORD").map(str::to_string),
        option_env!("RETROHYDRA_SCREENSCRAPER_DEVPASSWORD").map(str::to_string),
    ])?;
    Some((devid, devpassword))
}

fn resolve_dev_value(candidates: &[Option<String>]) -> Option<String> {
    // Take the first non-empty candidate so an empty runtime override does not
    // suppress a baked-in value.
    candidates
        .iter()
        .filter_map(|value| value.as_deref())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(|value| value.to_string())
}

pub fn system_id_for_platform(platform: &str) -> Option<u32> {
    Some(match platform {
        "genesis" | "megadrive" => 1,
        "nes" => 3,
        "snes" => 4,
        "gameboy" | "gb" => 9,
        "gbc" => 10,
        "gba" => 12,
        "gamecube" => 13,
        "n64" => 14,
        "nds" => 15,
        "dreamcast" => 23,
        "ps1" => 57,
        "ps2" => 58,
        "psp" => 61,
        "switch" => 225,
        _ => return None,
    })
}

fn game_node(value: &Value) -> Option<&Value> {
    value
        .get("response")
        .and_then(|response| response.get("jeu"))
        .or_else(|| value.get("jeu"))
}

fn game_nodes(value: &Value) -> Vec<&Value> {
    let Some(jeux) = value
        .get("response")
        .and_then(|response| response.get("jeux"))
        .or_else(|| value.get("jeux"))
    else {
        return game_node(value).into_iter().collect();
    };

    if let Some(items) = jeux.as_array() {
        return items.iter().collect();
    }
    if let Some(item) = jeux.get("jeu") {
        if let Some(items) = item.as_array() {
            return items.iter().collect();
        }
        return vec![item];
    }
    vec![jeux]
}

fn payload_from_game(
    game: &Value,
    options: &RequestOptions,
    match_kind: &str,
) -> Option<ScrapedGamePayload> {
    let provider_game_id = provider_game_id(game);
    let mut external_ids = BTreeMap::new();
    if let Some(id) = provider_game_id.as_ref() {
        external_ids.insert(PROVIDER.to_string(), id.clone());
    }

    let metadata = GameMetadata {
        release_year: release_year(game),
        developer: text_field(game, &["developpeur", "developer"], options),
        publisher: text_field(game, &["editeur", "publisher"], options),
        genres: text_list_field(game, &["genres", "genre"], options),
        tags: Vec::new(),
        players: text_field(game, &["joueurs", "players"], options),
        series: text_field(game, &["serie", "series"], options),
        external_ids,
    };

    let title = text_field(game, &["noms", "nom", "name", "titre"], options);
    if provider_game_id.is_none() && title.is_none() {
        return None;
    }

    Some(ScrapedGamePayload {
        provider: PROVIDER.to_string(),
        provider_game_id,
        title,
        description: text_field(game, &["synopsis", "description"], options),
        cover: media_cover(game, options),
        hero: None,
        logo: None,
        screenshots: Vec::new(),
        metadata,
    })
    .map(|mut payload| {
        if payload.metadata.tags.is_empty() {
            payload.metadata.tags.push(match_kind.to_string());
        }
        payload
    })
}

fn candidate_from_game(game: &Value, options: &RequestOptions) -> Option<ScrapeCandidate> {
    let payload = payload_from_game(game, options, "name")?;
    let provider_game_id = payload.provider_game_id.clone()?;
    Some(ScrapeCandidate {
        provider: PROVIDER.to_string(),
        provider_game_id,
        title: payload.title.unwrap_or_else(|| "Untitled".to_string()),
        platform: text_field(game, &["systeme", "system", "platform"], options),
        release_year: payload.metadata.release_year,
        developer: payload.metadata.developer,
        cover: payload.cover,
        match_kind: "name".to_string(),
    })
}

fn provider_game_id(game: &Value) -> Option<String> {
    first_string(game, &["id", "gameid", "jeuId"])
}

fn text_field(game: &Value, keys: &[&str], options: &RequestOptions) -> Option<String> {
    keys.iter()
        .find_map(|key| {
            game.get(*key)
                .and_then(|value| localized_text(value, options))
        })
        .filter(|value| !value.trim().is_empty())
}

fn text_list_field(game: &Value, keys: &[&str], options: &RequestOptions) -> Vec<String> {
    keys.iter()
        .find_map(|key| game.get(*key))
        .map(|value| match value {
            Value::Array(items) => items
                .iter()
                .filter_map(|item| localized_text(item, options))
                .filter(|value| !value.trim().is_empty())
                .take(6)
                .collect(),
            _ => localized_text(value, options).into_iter().collect(),
        })
        .unwrap_or_default()
}

fn localized_text(value: &Value, options: &RequestOptions) -> Option<String> {
    match value {
        Value::String(text) => clean_text(text),
        Value::Number(number) => clean_text(&number.to_string()),
        Value::Object(object) => ["text", "nom", "name", "value", "libelle"]
            .iter()
            .find_map(|key| {
                object
                    .get(*key)
                    .and_then(|item| localized_text(item, options))
            })
            .or_else(|| {
                preferred_region_codes(&options.region)
                    .iter()
                    .find_map(|region| {
                        object
                            .get(*region)
                            .and_then(|item| localized_text(item, options))
                    })
            }),
        Value::Array(items) => items
            .iter()
            .max_by_key(|item| localized_score(item, options))
            .and_then(|item| localized_text(item, options)),
        _ => None,
    }
}

fn localized_score(value: &Value, options: &RequestOptions) -> i32 {
    let region = first_string(value, &["region", "regions"]);
    let language = first_string(value, &["langue", "language", "lang"]);
    let mut score = 0;
    if let Some(region) = region {
        let region = region.to_lowercase();
        for (index, preferred) in preferred_region_codes(&options.region).iter().enumerate() {
            if region.contains(preferred) {
                score += 20 - index as i32;
                break;
            }
        }
    }
    if let Some(language) = language {
        if language.eq_ignore_ascii_case(&options.language) {
            score += 10;
        }
    }
    score
}

fn media_cover(game: &Value, options: &RequestOptions) -> Option<String> {
    let medias = game.get("medias")?.as_array()?;
    medias
        .iter()
        .filter_map(|media| {
            let url = first_string(media, &["url", "url_crc", "url_original"])?;
            let media_type = first_string(media, &["type", "media_type"]).unwrap_or_default();
            let mut score = match media_type.as_str() {
                "box-2D" | "box-2d" => 100,
                "box-3D" | "box-3d" => 70,
                "wheel" => 20,
                _ => 0,
            };
            score += localized_score(media, options);
            Some((score, url))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, url)| url)
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
}

fn release_year(game: &Value) -> Option<u16> {
    ["dates", "date", "releasedate", "datejeu"]
        .iter()
        .find_map(|key| game.get(*key).and_then(year_from_value))
}

fn year_from_value(value: &Value) -> Option<u16> {
    match value {
        Value::String(text) => year_from_text(text),
        Value::Number(number) => year_from_text(&number.to_string()),
        Value::Array(items) => items.iter().find_map(year_from_value),
        Value::Object(object) => object.values().find_map(year_from_value),
        _ => None,
    }
}

fn year_from_text(text: &str) -> Option<u16> {
    text.split(|char: char| !char.is_ascii_digit())
        .filter(|part| part.len() >= 4)
        .find_map(|part| {
            let year = part[..4].parse::<u16>().ok()?;
            (1950..=2100).contains(&year).then_some(year)
        })
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find_map(value_string)
        .and_then(|value| clean_text(&value))
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Object(object) => ["text", "nom", "name", "value"]
            .iter()
            .find_map(|key| object.get(*key).and_then(value_string)),
        _ => None,
    }
}

fn clean_text(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn preferred_region_codes(region: &str) -> Vec<&'static str> {
    match region {
        "us" => vec!["us", "ame", "wor", "ss", "eu", "jp"],
        "jp" => vec!["jp", "wor", "ss", "us", "eu"],
        "eu" => vec!["eu", "wor", "ss", "us", "jp"],
        _ => vec!["wor", "ss", "us", "eu", "jp"],
    }
}

fn response_success_is_false(value: &Value) -> bool {
    value
        .get("header")
        .and_then(|header| header.get("success"))
        .map(|success| match success {
            Value::Bool(value) => !value,
            Value::String(value) => value.eq_ignore_ascii_case("false") || value == "0",
            Value::Number(value) => value.as_i64() == Some(0),
            _ => false,
        })
        .unwrap_or(false)
}

fn response_looks_like_miss(value: &Value) -> bool {
    response_message(value)
        .map(|message| {
            let message = message.to_lowercase();
            message.contains("not found")
                || message.contains("non trouv")
                || message.contains("aucun")
                || message.contains("no game")
        })
        .unwrap_or(true)
}

fn response_message(value: &Value) -> Option<String> {
    value
        .get("header")
        .and_then(|header| {
            ["error", "message", "Erreur", "APIversion"]
                .iter()
                .find_map(|key| header.get(*key).and_then(value_string))
        })
        .or_else(|| {
            ["error", "message"]
                .iter()
                .find_map(|key| value.get(*key).and_then(value_string))
        })
}

#[cfg(test)]
mod tests {
    use super::resolve_dev_value;

    #[test]
    fn runtime_dev_value_wins_over_builtin() {
        assert_eq!(
            resolve_dev_value(&[Some(" runtime ".to_string()), Some("built-in".to_string())])
                .as_deref(),
            Some("runtime")
        );
    }

    #[test]
    fn empty_runtime_falls_back_to_builtin() {
        assert_eq!(
            resolve_dev_value(&[Some("  ".to_string()), Some(" built-in ".to_string())]).as_deref(),
            Some("built-in")
        );
    }

    #[test]
    fn no_dev_value_when_all_sources_blank() {
        assert!(resolve_dev_value(&[Some(String::new()), None]).is_none());
        assert!(resolve_dev_value(&[None, None]).is_none());
    }
}
