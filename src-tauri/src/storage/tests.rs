use tempfile::tempdir;

use super::rows::*;
use super::*;
use crate::schema::{
    AssetKind, GameArtwork, RepositoryAsset, RepositoryGame, RepositoryMetadata, SourceUri,
};

fn test_repo() -> RepositorySchema {
    RepositorySchema {
        metadata: RepositoryMetadata {
            id: "repo".to_string(),
            name: "Repo".to_string(),
            version: "1".to_string(),
            schema_version: 2,
            maintainer: None,
            homepage_url: None,
            license: None,
            trust_level: None,
            content_hash: None,
            updated_at: None,
        },
        system_files: vec![RepositoryAsset {
            id: "emu".to_string(),
            platform: "nes".to_string(),
            asset_kind: AssetKind::Emulator,
            display_name: "Emulator".to_string(),
            sources: vec![SourceUri::Http {
                url: "https://example.com/emulator.zip".to_string(),
                sha256: "a".repeat(64),
                size_bytes: None,
            }],
            install_hint: None,
            executable: true,
        }],
        catalog: vec![RepositoryGame {
            id: "game".to_string(),
            platform: "nes".to_string(),
            title: "Game".to_string(),
            description: None,
            cover_image_url: None,
            trailer_url: None,
            artwork: None,
            metadata: None,
            content_mode: None,
            setup_profile_id: None,
            downloads: vec![SourceUri::Magnet {
                uri: "magnet:?xt=urn:btih:abc".to_string(),
                info_hash: None,
                size_bytes: None,
            }],
            expected_extensions: vec![".nes".to_string()],
            required_system_file_ids: vec!["emu".to_string()],
            launch: None,
        }],
    }
}

fn catalog_view_with_artwork(artwork: Option<GameArtwork>) -> CatalogGameView {
    CatalogGameView {
        id: "repo::game".to_string(),
        source_id: "game".to_string(),
        repository_id: "repo".to_string(),
        repository_name: "Repo".to_string(),
        platform: "nes".to_string(),
        title: "Game".to_string(),
        description: None,
        cover_image_url: None,
        trailer_url: None,
        artwork,
        metadata: None,
        content_mode: None,
        setup_profile_id: None,
        downloads: Vec::new(),
        expected_extensions: vec![".nes".to_string()],
        required_system_file_ids: Vec::new(),
        launch: None,
    }
}

fn artwork_payload(hero: &str, logo: &str, screenshot: &str) -> ScrapedGamePayload {
    ScrapedGamePayload {
        provider: "test".to_string(),
        provider_game_id: Some("1".to_string()),
        title: Some("Game".to_string()),
        description: None,
        cover: None,
        hero: Some(hero.to_string()),
        logo: Some(logo.to_string()),
        screenshots: vec![screenshot.to_string()],
        metadata: empty_metadata(),
    }
}

#[test]
fn scraped_artwork_does_not_replace_source_artwork() {
    let mut view = catalog_view_with_artwork(Some(GameArtwork {
        cover: None,
        hero: Some("source-hero".to_string()),
        logo: Some("source-logo".to_string()),
        screenshots: vec!["source-shot".to_string()],
    }));
    let payload = artwork_payload("scraped-hero", "scraped-logo", "scraped-shot");

    apply_payload(&mut view, &payload, false);

    let artwork = view.artwork.unwrap();
    assert_eq!(artwork.hero.as_deref(), Some("source-hero"));
    assert_eq!(artwork.logo.as_deref(), Some("source-logo"));
    assert_eq!(artwork.screenshots, vec!["source-shot".to_string()]);
}

#[test]
fn override_artwork_replaces_existing_artwork() {
    let mut view = catalog_view_with_artwork(Some(GameArtwork {
        cover: None,
        hero: Some("source-hero".to_string()),
        logo: Some("source-logo".to_string()),
        screenshots: vec!["source-shot".to_string()],
    }));
    let payload = artwork_payload("override-hero", "override-logo", "override-shot");

    apply_payload(&mut view, &payload, true);

    let artwork = view.artwork.unwrap();
    assert_eq!(artwork.hero.as_deref(), Some("override-hero"));
    assert_eq!(artwork.logo.as_deref(), Some("override-logo"));
    assert_eq!(artwork.screenshots, vec!["override-shot".to_string()]);
}

#[test]
fn stores_and_reads_repository_catalog() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    let summary = store
        .store_repository("https://example.com/index.json", &test_repo())
        .unwrap();

    assert_eq!(summary.catalog_count, 1);
    assert_eq!(summary.system_file_count, 1);
    assert_eq!(store.list_repositories().unwrap().len(), 1);
    assert_eq!(
        store.get_catalog().unwrap()[0].required_system_file_ids[0],
        "repo::emu"
    );
    assert_eq!(
        store.get_catalog().unwrap()[0].expected_extensions,
        vec![".nes".to_string()]
    );
}

#[test]
fn batched_catalog_enrichment_matches_per_game_path() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository("https://example.com/index.json", &test_repo())
        .unwrap();

    // Seed scraped metadata (by name key) and an override for the one game.
    store
        .put_metadata(
            &metadata_name_key("nes", "Game"),
            "screenscraper",
            &artwork_payload("scraped-hero", "scraped-logo", "scraped-shot"),
            "name",
        )
        .unwrap();
    store
        .put_override(
            "repo::game",
            "provider-1",
            &artwork_payload("override-hero", "override-logo", "override-shot"),
        )
        .unwrap();

    // The batched get_catalog path must produce byte-identical enrichment to
    // the per-game get_game path.
    let batched = store.get_catalog().unwrap();
    let per_game = store.get_game("repo::game").unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&batched[0]).unwrap(),
        serde_json::to_value(&per_game).unwrap()
    );
}

#[test]
fn stores_and_updates_emulator_configs() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    let saved = store
        .upsert_emulator_config(
            "nes",
            Some("C:/Emulators/nes.exe"),
            "valid",
            Some("1.0.0"),
            Some("--fullscreen {game_path}"),
        )
        .unwrap();

    assert_eq!(saved.platform, "nes");
    assert_eq!(saved.status, "valid");
    assert_eq!(saved.exe_path.as_deref(), Some("C:/Emulators/nes.exe"));
    assert_eq!(store.list_emulator_configs().unwrap().len(), 1);

    let updated = store
        .upsert_emulator_config("nes", Some("C:/Missing/nes.exe"), "missing", None, None)
        .unwrap();
    assert_eq!(updated.status, "missing");
    assert_eq!(updated.version, None);

    assert!(store.delete_emulator_config("nes").unwrap());
    assert!(store.get_emulator_config("nes").unwrap().is_none());
}

#[test]
fn migrates_legacy_emulator_configs_to_default_profiles() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    let exe_path = dir.path().join("Mesen.exe");
    std::fs::write(&exe_path, b"exe").unwrap();
    let exe_path = exe_path.to_string_lossy().to_string();

    store
        .conn
        .execute(
            r#"
                INSERT INTO emulator_configs (
                  platform, exe_path, status, last_validated_at, version, launch_args_template
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
            rusqlite::params![
                "nes",
                exe_path.as_str(),
                "valid",
                "2026-06-09T00:00:00Z",
                "2.1.1",
                "{game_path}"
            ],
        )
        .unwrap();

    assert_eq!(store.migrate_legacy_emulator_configs().unwrap(), 1);
    assert_eq!(store.migrate_legacy_emulator_configs().unwrap(), 0);

    let profile = store
        .get_profile_emulator_config("nes-mesen")
        .unwrap()
        .unwrap();
    assert_eq!(profile.platform, "nes");
    assert_eq!(profile.status, "valid");
    assert_eq!(profile.version.as_deref(), Some("2.1.1"));
    assert_eq!(profile.launch_args_template.as_deref(), Some("{game_path}"));
    assert_eq!(
        store
            .get_emulator_exe_path("nes", Some("nes-mesen"))
            .unwrap()
            .as_deref(),
        Some(std::path::Path::new(&exe_path))
    );
}

#[test]
fn records_asset_installations() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    let saved = store
        .record_asset_installation(
            "repo::bios",
            Some("F:/System/bios.bin"),
            "ready",
            Some(&"c".repeat(64)),
            None,
        )
        .unwrap();

    assert_eq!(saved.status, "ready");
    assert_eq!(saved.target_path.as_deref(), Some("F:/System/bios.bin"));

    let updated = store
        .record_asset_installation(
            "repo::bios",
            Some("F:/System/bios.bin"),
            "corrupt",
            Some(&"d".repeat(64)),
            Some("SHA-256 mismatch"),
        )
        .unwrap();

    assert_eq!(updated.status, "corrupt");
    assert_eq!(updated.message.as_deref(), Some("SHA-256 mismatch"));
}

#[test]
fn stores_updates_and_completes_torrent_downloads() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    let started = store
        .upsert_torrent_download_start(
            "repo::game",
            "magnet:?xt=urn:btih:abc",
            "F:/Games/repo-game",
        )
        .unwrap();

    assert_eq!(started.status, "resolving");
    assert_eq!(started.progress_percent, 0.0);
    assert_eq!(started.torrent_id, None);

    let downloading = store
        .update_torrent_handle("repo::game", 7, "downloading")
        .unwrap();
    assert_eq!(downloading.torrent_id, Some(7));
    assert_eq!(downloading.status, "downloading");

    let progressed = store
        .update_torrent_progress("repo::game", "downloading", 42.5, 425, 1000, 12, 3, 4)
        .unwrap();
    assert_eq!(progressed.progress_percent, 42.5);
    assert_eq!(progressed.downloaded_bytes, 425);
    assert_eq!(progressed.total_bytes, 1000);
    assert_eq!(progressed.peers_count, 4);

    let completed = store
        .mark_torrent_completed("repo::game", "F:/Games/repo-game", 1000, 1000)
        .unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.progress_percent, 100.0);
    assert!(completed.completed_at.is_some());

    let legacy = store.get_download("repo::game").unwrap().unwrap();
    assert_eq!(legacy.subject_type, "game");
    assert_eq!(legacy.status, "ready");
    assert_eq!(legacy.local_path.as_deref(), Some("F:/Games/repo-game"));
}

#[test]
fn records_direct_completed_downloads_for_launcher_state() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    let (download, torrent) = store
        .record_direct_game_download_completed(
            "repo::game",
            "bundled",
            "F:/Games/game.nes",
            &"b".repeat(64),
            24_592,
        )
        .unwrap();

    assert_eq!(download.status, "ready");
    assert_eq!(download.local_path.as_deref(), Some("F:/Games/game.nes"));
    assert_eq!(torrent.status, "completed");
    assert_eq!(torrent.magnet_uri, "direct:bundled");
    assert_eq!(torrent.save_dir, "F:/Games/game.nes");
    assert_eq!(torrent.progress_percent, 100.0);
    assert_eq!(torrent.downloaded_bytes, 24_592);
    assert_eq!(torrent.total_bytes, 24_592);
    assert!(torrent.completed_at.is_some());
}

#[test]
fn records_direct_download_errors_for_retry() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    let started = store
        .record_direct_game_download_started("repo::game", "http", "F:/Games/game.nes", 24_592)
        .unwrap();
    assert_eq!(started.status, "downloading");
    assert_eq!(started.magnet_uri, "direct:http");
    assert_eq!(started.total_bytes, 24_592);

    let failed = store
        .record_direct_game_download_failed(
            "repo::game",
            "http",
            "F:/Games/game.nes",
            24_592,
            "Source returned an error",
        )
        .unwrap();
    assert_eq!(failed.status, "error");
    assert_eq!(
        failed.error_message.as_deref(),
        Some("Source returned an error")
    );

    let legacy = store.get_download("repo::game").unwrap().unwrap();
    assert_eq!(legacy.status, "error");
    assert_eq!(legacy.message.as_deref(), Some("Source returned an error"));
}

#[test]
fn lists_startup_torrent_downloads() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    store
        .upsert_torrent_download_start("active", "magnet:?xt=urn:btih:abc", "F:/Games/a")
        .unwrap();
    store
        .upsert_torrent_download_start("paused", "magnet:?xt=urn:btih:def", "F:/Games/p")
        .unwrap();
    store.set_torrent_status("paused", "paused", None).unwrap();
    store
        .upsert_torrent_download_start("cancelled", "magnet:?xt=urn:btih:ghi", "F:/Games/c")
        .unwrap();
    store
        .set_torrent_status("cancelled", "cancelled", None)
        .unwrap();

    let game_ids = store
        .list_startup_torrent_downloads()
        .unwrap()
        .into_iter()
        .map(|record| record.game_id)
        .collect::<Vec<_>>();

    assert!(game_ids.contains(&"active".to_string()));
    assert!(game_ids.contains(&"paused".to_string()));
    assert!(!game_ids.contains(&"cancelled".to_string()));
}

#[test]
fn lists_torrent_downloads_with_actionable_records_first() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    store
        .upsert_torrent_download_start("completed", "magnet:?xt=urn:btih:aaa", "F:/Games/c")
        .unwrap();
    store
        .mark_torrent_completed("completed", "F:/Games/c", 100, 100)
        .unwrap();
    store
        .upsert_torrent_download_start("cancelled", "magnet:?xt=urn:btih:bbb", "F:/Games/x")
        .unwrap();
    store
        .set_torrent_status("cancelled", "cancelled", None)
        .unwrap();
    store
        .upsert_torrent_download_start("paused", "magnet:?xt=urn:btih:ccc", "F:/Games/p")
        .unwrap();
    store.set_torrent_status("paused", "paused", None).unwrap();
    store
        .upsert_torrent_download_start("error", "magnet:?xt=urn:btih:ddd", "F:/Games/e")
        .unwrap();
    store
        .set_torrent_status("error", "error", Some("No peers found"))
        .unwrap();
    store
        .upsert_torrent_download_start("active", "magnet:?xt=urn:btih:eee", "F:/Games/a")
        .unwrap();

    let game_ids = store
        .list_torrent_downloads()
        .unwrap()
        .into_iter()
        .map(|record| record.game_id)
        .collect::<Vec<_>>();

    let position = |game_id: &str| game_ids.iter().position(|id| id == game_id).unwrap();
    assert!(position("active") < position("error"));
    assert!(position("error") < position("paused"));
    assert!(position("paused") < position("completed"));
    assert!(position("completed") < position("cancelled"));
}
