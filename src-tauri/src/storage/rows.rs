use super::*;

pub(super) fn map_scrape_state_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScrapeStateView> {
    let candidates_json: Option<String> = row.get(3)?;
    let candidates = candidates_json
        .as_deref()
        .filter(|json| !json.trim().is_empty() && json.trim() != "null")
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?
        .unwrap_or_default();

    Ok(ScrapeStateView {
        game_id: row.get(0)?,
        status: row.get(1)?,
        match_kind: row.get(2)?,
        candidates,
        message: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub(super) fn enrich_with_scraped(
    view: &mut CatalogGameView,
    scraped: Option<&ScrapedGamePayload>,
    steamgriddb: Option<&ScrapedGamePayload>,
    override_: Option<&ScrapedGamePayload>,
) {
    if let Some(payload) = scraped {
        apply_payload(view, payload, false);
    }
    if let Some(payload) = steamgriddb {
        apply_payload(view, payload, false);
    }
    if let Some(payload) = override_ {
        apply_payload(view, payload, true);
    }
}

pub(super) fn apply_payload(
    view: &mut CatalogGameView,
    payload: &ScrapedGamePayload,
    replace_existing: bool,
) {
    if let Some(description) = payload
        .description
        .as_deref()
        .filter(|value| !is_blank(value))
    {
        if replace_existing || view.description.as_deref().map(is_blank).unwrap_or(true) {
            view.description = Some(description.to_string());
        }
    }

    if let Some(cover) = payload.cover.as_deref().filter(|value| !is_blank(value)) {
        let source_has_cover = view
            .artwork
            .as_ref()
            .and_then(|artwork| artwork.cover.as_deref())
            .map(|value| !is_blank(value))
            .unwrap_or(false)
            || view
                .cover_image_url
                .as_deref()
                .map(|value| !is_blank(value))
                .unwrap_or(false);
        if replace_existing || !source_has_cover {
            let artwork = view.artwork.get_or_insert_with(empty_artwork);
            artwork.cover = Some(cover.to_string());
            if replace_existing
                || view
                    .cover_image_url
                    .as_deref()
                    .map(is_blank)
                    .unwrap_or(true)
            {
                view.cover_image_url = Some(cover.to_string());
            }
        }
    }

    if let Some(hero) = payload.hero.as_deref().filter(|value| !is_blank(value)) {
        let source_has_hero = view
            .artwork
            .as_ref()
            .and_then(|artwork| artwork.hero.as_deref())
            .map(|value| !is_blank(value))
            .unwrap_or(false);
        if replace_existing || !source_has_hero {
            let artwork = view.artwork.get_or_insert_with(empty_artwork);
            artwork.hero = Some(hero.to_string());
        }
    }

    if let Some(logo) = payload.logo.as_deref().filter(|value| !is_blank(value)) {
        let source_has_logo = view
            .artwork
            .as_ref()
            .and_then(|artwork| artwork.logo.as_deref())
            .map(|value| !is_blank(value))
            .unwrap_or(false);
        if replace_existing || !source_has_logo {
            let artwork = view.artwork.get_or_insert_with(empty_artwork);
            artwork.logo = Some(logo.to_string());
        }
    }

    let screenshots = payload
        .screenshots
        .iter()
        .filter(|value| !is_blank(value))
        .cloned()
        .collect::<Vec<_>>();
    if !screenshots.is_empty() {
        let source_has_screenshots = view
            .artwork
            .as_ref()
            .map(|artwork| artwork.screenshots.iter().any(|value| !is_blank(value)))
            .unwrap_or(false);
        if replace_existing || !source_has_screenshots {
            let artwork = view.artwork.get_or_insert_with(empty_artwork);
            artwork.screenshots = screenshots;
        }
    }

    merge_metadata(view, &payload.metadata, replace_existing);
}

pub(super) fn merge_metadata(
    view: &mut CatalogGameView,
    scraped: &GameMetadata,
    replace_existing: bool,
) {
    let metadata = view.metadata.get_or_insert_with(empty_metadata);

    if replace_existing || metadata.release_year.is_none() {
        metadata.release_year = scraped.release_year.or(metadata.release_year);
    }
    merge_string_field(
        &mut metadata.developer,
        scraped.developer.as_deref(),
        replace_existing,
    );
    merge_string_field(
        &mut metadata.publisher,
        scraped.publisher.as_deref(),
        replace_existing,
    );
    merge_string_field(
        &mut metadata.players,
        scraped.players.as_deref(),
        replace_existing,
    );
    merge_string_field(
        &mut metadata.series,
        scraped.series.as_deref(),
        replace_existing,
    );

    if replace_existing {
        if !scraped.genres.is_empty() {
            metadata.genres = scraped.genres.clone();
        }
        if !scraped.tags.is_empty() {
            metadata.tags = scraped.tags.clone();
        }
    } else {
        if metadata.genres.is_empty() {
            metadata.genres = scraped.genres.clone();
        }
        if metadata.tags.is_empty() {
            metadata.tags = scraped.tags.clone();
        }
    }

    for (key, value) in &scraped.external_ids {
        if replace_existing || !metadata.external_ids.contains_key(key) {
            metadata.external_ids.insert(key.clone(), value.clone());
        }
    }
}

pub(super) fn merge_string_field(
    target: &mut Option<String>,
    incoming: Option<&str>,
    replace_existing: bool,
) {
    let Some(incoming) = incoming.filter(|value| !is_blank(value)) else {
        return;
    };
    if replace_existing || target.as_deref().map(is_blank).unwrap_or(true) {
        *target = Some(incoming.to_string());
    }
}

pub(super) fn empty_artwork() -> GameArtwork {
    GameArtwork {
        cover: None,
        hero: None,
        logo: None,
        screenshots: Vec::new(),
    }
}

pub(super) fn empty_metadata() -> GameMetadata {
    GameMetadata {
        release_year: None,
        developer: None,
        publisher: None,
        genres: Vec::new(),
        tags: Vec::new(),
        players: None,
        series: None,
        external_ids: std::collections::BTreeMap::new(),
    }
}

pub(super) fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}

pub(super) fn map_game_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CatalogGameView> {
    let artwork_json: Option<String> = row.get(9)?;
    let metadata_json: Option<String> = row.get(10)?;
    let downloads_json: String = row.get(13)?;
    let expected_extensions_json: String = row.get(14)?;
    let required_json: String = row.get(15)?;
    let launch_json: Option<String> = row.get(16)?;
    Ok(CatalogGameView {
        id: row.get(0)?,
        source_id: row.get(1)?,
        repository_id: row.get(2)?,
        repository_name: row.get(3)?,
        platform: row.get(4)?,
        title: row.get(5)?,
        description: row.get(6)?,
        cover_image_url: row.get(7)?,
        trailer_url: row.get(8)?,
        artwork: artwork_json
            .as_deref()
            .filter(|json| !json.trim().is_empty() && json.trim() != "null")
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        metadata: metadata_json
            .as_deref()
            .filter(|json| !json.trim().is_empty() && json.trim() != "null")
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        content_mode: row.get(11)?,
        setup_profile_id: row.get(12)?,
        downloads: serde_json::from_str(&downloads_json).unwrap_or_default(),
        expected_extensions: serde_json::from_str(&expected_extensions_json).unwrap_or_default(),
        required_system_file_ids: serde_json::from_str(&required_json).unwrap_or_default(),
        launch: launch_json
            .as_deref()
            .filter(|json| !json.trim().is_empty() && json.trim() != "null")
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    16,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
    })
}

pub(super) fn map_repository_summary_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RepositorySummary> {
    Ok(RepositorySummary {
        id: row.get(0)?,
        name: row.get(1)?,
        version: row.get(2)?,
        url: row.get(3)?,
        connected_at: row.get(4)?,
        maintainer: row.get(5)?,
        homepage_url: row.get(6)?,
        license: row.get(7)?,
        trust_level: row.get(8)?,
        content_hash: row.get(9)?,
        last_refreshed_at: row.get(10)?,
        catalog_count: row.get::<_, i64>(11)? as usize,
        system_file_count: row.get::<_, i64>(12)? as usize,
        has_executable_assets: row.get::<_, i64>(13)? == 1,
    })
}

pub(super) fn map_download_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRecord> {
    Ok(DownloadRecord {
        subject_id: row.get(0)?,
        subject_type: row.get(1)?,
        status: row.get(2)?,
        local_path: row.get(3)?,
        sha256: row.get(4)?,
        message: row.get(5)?,
        source: row.get(6)?,
        magnet_uri: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub(super) fn map_asset_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetView> {
    let asset_kind_json = format!("\"{}\"", row.get::<_, String>(4)?);
    let sources_json: String = row.get(6)?;
    let install_hint_json: Option<String> = row.get(7)?;
    let install_hint = install_hint_json
        .as_deref()
        .filter(|json| !json.trim().is_empty() && json.trim() != "null")
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(AssetView {
        id: row.get(0)?,
        source_id: row.get(1)?,
        repository_id: row.get(2)?,
        platform: row.get(3)?,
        asset_kind: serde_json::from_str(&asset_kind_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        display_name: row.get(5)?,
        sources: serde_json::from_str(&sources_json).unwrap_or_default(),
        install_hint,
        executable: row.get::<_, i64>(8)? == 1,
    })
}

pub(super) fn map_emulator_config_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmulatorConfig> {
    Ok(EmulatorConfig {
        platform: row.get(0)?,
        exe_path: row.get(1)?,
        status: row.get(2)?,
        last_validated_at: row.get(3)?,
        version: row.get(4)?,
        launch_args_template: row.get(5)?,
    })
}

pub(super) fn map_profile_emulator_config_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProfileEmulatorConfig> {
    Ok(ProfileEmulatorConfig {
        profile_id: row.get(0)?,
        platform: row.get(1)?,
        exe_path: row.get(2)?,
        status: row.get(3)?,
        last_validated_at: row.get(4)?,
        version: row.get(5)?,
        launch_args_template: row.get(6)?,
    })
}

pub(super) fn emulator_config_from_profile(config: ProfileEmulatorConfig) -> EmulatorConfig {
    EmulatorConfig {
        platform: config.platform,
        exe_path: config.exe_path,
        status: config.status,
        last_validated_at: config.last_validated_at,
        version: config.version,
        launch_args_template: config.launch_args_template,
    }
}

pub(super) fn map_asset_installation_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AssetInstallation> {
    Ok(AssetInstallation {
        asset_id: row.get(0)?,
        target_path: row.get(1)?,
        status: row.get(2)?,
        sha256: row.get(3)?,
        verified_at: row.get(4)?,
        message: row.get(5)?,
    })
}

pub(super) fn map_profile_system_file_import_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProfileSystemFileImport> {
    Ok(ProfileSystemFileImport {
        profile_id: row.get(0)?,
        requirement_id: row.get(1)?,
        target_path: row.get(2)?,
        status: row.get(3)?,
        sha256: row.get(4)?,
        verified_at: row.get(5)?,
        message: row.get(6)?,
    })
}

pub(super) fn map_torrent_download_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TorrentDownloadRecord> {
    Ok(TorrentDownloadRecord {
        game_id: row.get(0)?,
        magnet_uri: row.get(1)?,
        save_dir: row.get(2)?,
        status: row.get(3)?,
        progress_percent: row.get(4)?,
        downloaded_bytes: i64_to_u64(row.get(5)?),
        total_bytes: i64_to_u64(row.get(6)?),
        download_speed_bytes_per_sec: i64_to_u64(row.get(7)?),
        upload_speed_bytes_per_sec: i64_to_u64(row.get(8)?),
        peers_count: i64_to_usize(row.get(9)?),
        torrent_id: row.get(10)?,
        error_message: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        completed_at: row.get(14)?,
    })
}

pub(super) fn u64_to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

pub(super) fn i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

pub(super) fn i64_to_usize(value: i64) -> usize {
    value.max(0) as usize
}
