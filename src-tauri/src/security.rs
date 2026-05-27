use std::collections::HashSet;
use std::path::{Component, Path};
use url::Url;

use crate::schema::{RepositorySchema, SourceUri};

const GLOBAL_ID_SEPARATOR: &str = "::";
const SUPPORTED_PLATFORMS: &[&str] = &[
    "switch",
    "nes",
    "snes",
    "n64",
    "gba",
    "nds",
    "gamecube",
    "wii",
    "ps1",
    "ps2",
    "ps3",
    "psp",
    "genesis",
    "saturn",
    "dreamcast",
];

pub fn global_id(repository_id: &str, source_id: &str) -> String {
    format!("{repository_id}{GLOBAL_ID_SEPARATOR}{source_id}")
}

pub fn validate_repository_url(input: &str, allow_dev_http: bool) -> Result<Url, String> {
    let parsed = Url::parse(input).map_err(|error| format!("Invalid repository URL: {error}"))?;
    validate_http_url(&parsed, allow_dev_http)?;
    Ok(parsed)
}

pub fn validate_http_url(url: &Url, allow_dev_http: bool) -> Result<(), String> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if allow_dev_http && is_loopback_host(url.host_str()) => Ok(()),
        "http" => Err(
            "Only HTTPS repository and asset URLs are allowed outside local development."
                .to_string(),
        ),
        scheme => Err(format!("Unsupported URL scheme: {scheme}")),
    }
}

pub fn validate_repository_schema(
    repo: &RepositorySchema,
    allow_dev_http: bool,
) -> Result<(), String> {
    if repo.metadata.schema_version != 2 {
        return Err("Unsupported repository schemaVersion. Expected 2.".to_string());
    }
    validate_id("repository id", &repo.metadata.id)?;
    validate_non_empty("repository name", &repo.metadata.name)?;
    validate_non_empty("repository version", &repo.metadata.version)?;
    if let Some(homepage_url) = &repo.metadata.homepage_url {
        validate_http_url(
            &Url::parse(homepage_url).map_err(|error| format!("Invalid homepageUrl: {error}"))?,
            allow_dev_http,
        )?;
    }
    if let Some(trust_level) = &repo.metadata.trust_level {
        validate_trust_level(trust_level)?;
    }
    if let Some(content_hash) = &repo.metadata.content_hash {
        validate_sha256(content_hash)?;
    }

    let mut asset_ids = HashSet::new();
    for asset in &repo.system_files {
        validate_id("asset id", &asset.id)?;
        validate_platform(&asset.platform)?;
        validate_non_empty("asset displayName", &asset.display_name)?;
        if !asset_ids.insert(asset.id.as_str()) {
            return Err(format!("Duplicate asset id: {}", asset.id));
        }
        validate_sources(&asset.sources, allow_dev_http)?;
        if let Some(hint) = &asset.install_hint {
            if let Some(relative_path) = &hint.relative_path {
                validate_relative_path(relative_path)?;
            }
        }
        if asset.executable
            && !asset
                .sources
                .iter()
                .any(|source| matches!(source, SourceUri::Http { .. }))
        {
            return Err(format!(
                "Executable asset {} must have at least one HTTP source with sha256.",
                asset.id
            ));
        }
    }

    let mut game_ids = HashSet::new();
    for game in &repo.catalog {
        validate_id("game id", &game.id)?;
        validate_platform(&game.platform)?;
        validate_non_empty("game title", &game.title)?;
        validate_expected_extensions(&game.expected_extensions)?;
        if !game_ids.insert(game.id.as_str()) {
            return Err(format!("Duplicate game id: {}", game.id));
        }
        validate_sources(&game.downloads, allow_dev_http)?;
        if let Some(url) = &game.cover_image_url {
            validate_http_url(
                &Url::parse(url).map_err(|error| format!("Invalid coverImageUrl: {error}"))?,
                allow_dev_http,
            )?;
        }
        if let Some(url) = &game.trailer_url {
            validate_http_url(
                &Url::parse(url).map_err(|error| format!("Invalid trailerUrl: {error}"))?,
                allow_dev_http,
            )?;
        }
        for required_id in &game.required_system_file_ids {
            if !asset_ids.contains(required_id.as_str()) {
                return Err(format!(
                    "Game {} requires unknown system file {}",
                    game.id, required_id
                ));
            }
        }
    }

    Ok(())
}

pub fn validate_platform(platform: &str) -> Result<(), String> {
    if SUPPORTED_PLATFORMS.contains(&platform) {
        return Ok(());
    }

    Err(format!("Unsupported platform: {platform}"))
}

fn validate_expected_extensions(extensions: &[String]) -> Result<(), String> {
    if extensions.is_empty() {
        return Err("Each game must include at least one expected extension.".to_string());
    }

    for extension in extensions {
        validate_expected_extension(extension)?;
    }

    Ok(())
}

fn validate_expected_extension(extension: &str) -> Result<(), String> {
    let valid = extension.starts_with('.')
        && extension.len() > 1
        && extension
            .chars()
            .skip(1)
            .all(|char| char.is_ascii_alphanumeric());

    if valid {
        return Ok(());
    }

    Err(format!(
        "Invalid expected extension: {extension}. Extensions must look like .nsp"
    ))
}

fn validate_sources(sources: &[SourceUri], allow_dev_http: bool) -> Result<(), String> {
    if sources.is_empty() {
        return Err("Each asset or game must include at least one source.".to_string());
    }

    for source in sources {
        match source {
            SourceUri::Http { url, sha256, .. } => {
                validate_http_url(
                    &Url::parse(url).map_err(|error| format!("Invalid source URL: {error}"))?,
                    allow_dev_http,
                )?;
                validate_sha256(sha256)?;
            }
            SourceUri::Magnet { uri, .. } => {
                if !uri.starts_with("magnet:?") {
                    return Err("Magnet sources must start with magnet:?".to_string());
                }
            }
            SourceUri::UserProvided { sha256, .. } => {
                if let Some(sha256) = sha256 {
                    validate_sha256(sha256)?;
                }
            }
        }
    }

    Ok(())
}

fn validate_trust_level(value: &str) -> Result<(), String> {
    match value {
        "official" | "community" | "unknown" => Ok(()),
        _ => Err(format!(
            "Invalid repository trustLevel: {value}. Expected official, community, or unknown."
        )),
    }
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() == 64 && value.chars().all(|char| char.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err("HTTP sources must include a 64-character SHA-256 hash.".to_string())
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("installHint.relativePath cannot be empty.".to_string());
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err("installHint.relativePath must be relative.".to_string());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(
                    "installHint.relativePath cannot escape its target directory.".to_string(),
                )
            }
        }
    }

    Ok(())
}

fn validate_id(label: &str, value: &str) -> Result<(), String> {
    validate_non_empty(label, value)?;
    if value.contains(GLOBAL_ID_SEPARATOR) {
        return Err(format!("{label} cannot contain {GLOBAL_ID_SEPARATOR}"));
    }
    Ok(())
}

fn validate_non_empty(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} cannot be empty."));
    }
    Ok(())
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(
        host,
        Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https_production_urls() {
        assert!(validate_repository_url("file:///repo.json", false).is_err());
        assert!(validate_repository_url("data:{}", false).is_err());
        assert!(validate_repository_url("http://example.com/repo.json", false).is_err());
    }

    #[test]
    fn allows_local_http_only_in_dev() {
        assert!(validate_repository_url("http://localhost:3000/repo.json", true).is_ok());
        assert!(validate_repository_url("http://localhost:3000/repo.json", false).is_err());
    }

    #[test]
    fn rejects_install_hints_that_escape_target_directory() {
        assert!(validate_relative_path("bios/scph1001.bin").is_ok());
        assert!(validate_relative_path("../secret.bin").is_err());
        assert!(validate_relative_path("/secret.bin").is_err());
    }
}
