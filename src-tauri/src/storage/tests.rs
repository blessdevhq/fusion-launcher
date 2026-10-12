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

fn cover_payload(cover: &str) -> ScrapedGamePayload {
    ScrapedGamePayload {
        provider: "steamgriddb".to_string(),
        provider_game_id: Some("1".to_string()),
        title: Some("Game".to_string()),
        description: None,
        cover: Some(cover.to_string()),
        hero: None,
        logo: None,
        screenshots: Vec::new(),
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
fn scraped_cover_replaces_placeholder_source_artwork() {
    let mut view = catalog_view_with_artwork(Some(GameArtwork {
        cover: Some("https://images.igdb.com/...png".to_string()),
        hero: Some("https://images.igdb.com/...jpg".to_string()),
        logo: None,
        screenshots: Vec::new(),
    }));
    view.cover_image_url = Some("https://images.igdb.com/...png".to_string());

    apply_payload(
        &mut view,
        &cover_payload("https://cdn2.steamgriddb.com/grid/real.png"),
        false,
    );

    let artwork = view.artwork.unwrap();
    assert_eq!(
        artwork.cover.as_deref(),
        Some("https://cdn2.steamgriddb.com/grid/real.png")
    );
    assert_eq!(
        view.cover_image_url.as_deref(),
        Some("https://cdn2.steamgriddb.com/grid/real.png")
    );
}

#[test]
fn placeholder_override_artwork_does_not_replace_scraped_cover() {
    let mut view = catalog_view_with_artwork(Some(GameArtwork {
        cover: Some("https://images.igdb.com/...png".to_string()),
        hero: None,
        logo: None,
        screenshots: Vec::new(),
    }));
    view.cover_image_url = Some("https://images.igdb.com/...png".to_string());

    apply_payload(
        &mut view,
        &cover_payload("https://cdn2.steamgriddb.com/grid/real.png"),
        false,
    );
    apply_payload(
        &mut view,
        &cover_payload("https://images.igdb.com/...png"),
        true,
    );

    let artwork = view.artwork.unwrap();
    assert_eq!(
        artwork.cover.as_deref(),
        Some("https://cdn2.steamgriddb.com/grid/real.png")
    );
    assert_eq!(
        view.cover_image_url.as_deref(),
        Some("https://cdn2.steamgriddb.com/grid/real.png")
    );
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
fn missing_artwork_backfill_retries_recoverable_scrape_states() {
    for (status, message) in [
        (
            "failed",
            "SteamGridDB artwork lookup failed: temporary network error",
        ),
        (
            "skipped",
            "ScreenScraper credentials are not configured. Configure SteamGridDB to fetch artwork only.",
        ),
    ] {
        let dir = tempdir().unwrap();
        let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
        store
            .store_repository("https://example.com/index.json", &test_repo())
            .unwrap();
        store
            .set_scrape_state("repo::game", status, Some("artwork"), &[], Some(message))
            .unwrap();

        let queued = store.mark_games_missing_artwork_pending_for_scrape(80).unwrap();

        assert_eq!(queued, 1, "{status} should be retried");
        let state = store.get_scrape_state("repo::game").unwrap().unwrap();
        assert_eq!(state.status, "pending");
        assert_eq!(state.message.as_deref(), Some("Queued for automatic cover art."));
    }
}

#[test]
fn missing_artwork_backfill_does_not_retry_confirmed_not_found_state() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository("https://example.com/index.json", &test_repo())
        .unwrap();
    store
        .set_scrape_state(
            "repo::game",
            "skipped",
            Some("artwork"),
            &[],
            Some("ScreenScraper credentials are not configured. SteamGridDB did not find artwork for this game."),
        )
        .unwrap();

    let queued = store
        .mark_games_missing_artwork_pending_for_scrape(80)
        .unwrap();

    assert_eq!(queued, 0);
    let state = store.get_scrape_state("repo::game").unwrap().unwrap();
    assert_eq!(state.status, "skipped");
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
fn deletes_only_the_requested_profile_emulator_config() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    store
        .upsert_profile_emulator_config(
            "ps2-pcsx2",
            "ps2",
            Some("C:/Managed/pcsx2-qt.exe"),
            "valid",
            Some("nightly"),
            Some("-fullscreen -- {game_path}"),
        )
        .unwrap();
    store
        .upsert_profile_emulator_config(
            "ps2-manual",
            "ps2",
            Some("D:/Tools/PCSX2/pcsx2-qt.exe"),
            "valid",
            None,
            Some("-fullscreen -- \"{game_path}\""),
        )
        .unwrap();

    assert!(store.delete_profile_emulator_config("ps2-pcsx2").unwrap());

    assert!(store
        .get_profile_emulator_config("ps2-pcsx2")
        .unwrap()
        .is_none());
    assert!(store
        .get_profile_emulator_config("ps2-manual")
        .unwrap()
        .is_some());
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
fn sanitizes_poisoned_manifest_emulator_records_across_all_tables() {
    let dir = tempdir().unwrap();
    let store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();

    // Insert poisoned records in legacy and profile tables as well as app_config
    store
        .conn
        .execute(
            r#"
            INSERT INTO emulator_configs (platform, exe_path, status, last_validated_at, version, launch_args_template)
            VALUES ('snes', 'C:/bad/snes.exe', 'valid', '2026-06-09T00:00:00Z', 'manifest', '{game_path}')
            "#,
            [],
        )
        .unwrap();

    store
        .conn
        .execute(
            r#"
            INSERT INTO profile_emulator_configs (profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template)
            VALUES ('snes-mesen', 'snes', 'C:/bad/snes.exe', 'valid', '2026-06-09T00:00:00Z', 'manifest', '{game_path}')
            "#,
            [],
        )
        .unwrap();

    store
        .conn
        .execute(
            r#"
            INSERT INTO app_config (key, value, updated_at)
            VALUES ('manifest_emulator:game1', '{"url":"https://evil.com/core.zip"}', '2026-06-09T00:00:00Z')
            "#,
            [],
        )
        .unwrap();

    let cleaned = store.sanitize_manifest_emulator_records().unwrap();
    assert_eq!(cleaned, 3);

    // Repeated run is idempotent
    let second_run = store.sanitize_manifest_emulator_records().unwrap();
    assert_eq!(second_run, 0);

    let poisoned_legacy: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM emulator_configs WHERE version = 'manifest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(poisoned_legacy, 0);

    let poisoned_profile: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM profile_emulator_configs WHERE version = 'manifest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(poisoned_profile, 0);

    let poisoned_config: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM app_config WHERE key LIKE 'manifest_emulator:%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(poisoned_config, 0);
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

#[test]
fn sqlite_initializes_with_wal_and_busy_timeout() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("fusion-launcher.db");
    let store = RepositoryStore::open(&db_path).unwrap();

    let journal_mode: String = store
        .conn
        .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_lowercase(), "wal");

    let busy_timeout: i64 = store
        .conn
        .query_row("PRAGMA busy_timeout;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy_timeout, 5000);
}

#[test]
fn manifest_emulator_records_preserved_across_store_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("fusion-launcher.db");

    // First open: creates schema, runs initial migration, sets user_version = 1
    {
        let store = RepositoryStore::open(&db_path).unwrap();
        store
            .set_config(
                "manifest_emulator:switch-game-1",
                r#"{"url":"https://example.com/eden.zip","archive_sha256":"abc"}"#,
            )
            .unwrap();
    }

    // Second open (launcher restart): user_version is already 1, must NOT wipe manifest_emulator config
    {
        let store = RepositoryStore::open(&db_path).unwrap();
        let value = store.get_config("manifest_emulator:switch-game-1").unwrap();
        assert!(
            value.is_some(),
            "manifest_emulator record must persist across store reopen"
        );
    }
}

#[test]
fn migration_from_v0_legacy_database_with_mixed_records() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("legacy-fusion.db");

    // 1. Manually construct a raw legacy SQLite database at user_version = 0
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            PRAGMA user_version = 0;

            CREATE TABLE emulator_configs (
                platform TEXT PRIMARY KEY,
                exe_path TEXT,
                status TEXT NOT NULL,
                last_validated_at TEXT,
                version TEXT,
                launch_args_template TEXT
            );

            CREATE TABLE profile_emulator_configs (
                profile_id TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                exe_path TEXT,
                status TEXT NOT NULL,
                last_validated_at TEXT,
                version TEXT,
                launch_args_template TEXT
            );

            CREATE TABLE app_config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE repositories (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                schema_version INTEGER NOT NULL,
                trust_level TEXT NOT NULL DEFAULT 'unknown',
                connected_at TEXT NOT NULL,
                raw_json TEXT NOT NULL
            );

            -- Valid normal profile config that must be preserved
            INSERT INTO profile_emulator_configs
                (profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template)
            VALUES
                ('nes-mesen', 'nes', 'C:/Games/Mesen.exe', 'valid', '2026-06-01T00:00:00Z', '2.1.1', '{game_path}');

            -- Poisoned legacy record that must be wiped
            INSERT INTO emulator_configs
                (platform, exe_path, status, last_validated_at, version, launch_args_template)
            VALUES
                ('switch', 'C:/malicious/evil.exe', 'valid', '2026-06-01T00:00:00Z', 'manifest', '{game_path}');

            -- Poisoned profile record that must be wiped
            INSERT INTO profile_emulator_configs
                (profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template)
            VALUES
                ('switch-manual', 'switch', 'C:/malicious/evil.exe', 'valid', '2026-06-01T00:00:00Z', 'manifest', '{game_path}');

            -- Poisoned app_config key that must be wiped
            INSERT INTO app_config (key, value, updated_at)
            VALUES ('manifest_emulator:poisoned-game', '{"url":"https://evil.com/core.zip"}', '2026-06-01T00:00:00Z');

            -- Normal app_config key that must be preserved
            INSERT INTO app_config (key, value, updated_at)
            VALUES ('download_root', 'F:/CustomGames', '2026-06-01T00:00:00Z');

            -- External repo falsely claiming 'official' trust level (must be demoted to 'community')
            INSERT INTO repositories (id, url, name, version, schema_version, trust_level, connected_at, raw_json)
            VALUES ('attacker-repo', 'https://attacker.com/repo.json', 'Attacker Repo', '1.0.0', 1, 'official', '2026-01-01', '{}');

            -- Builtin demo repo claiming 'official' (must remain 'official')
            INSERT INTO repositories (id, url, name, version, schema_version, trust_level, connected_at, raw_json)
            VALUES ('builtin-repo', 'fusion-launcher://builtin/demo-repository.json', 'Builtin Demo', '1.0.0', 1, 'official', '2026-01-01', '{}');
            "#,
        )
        .unwrap();

        let v: i32 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, 0);
    }

    // 2. Open via RepositoryStore::open - executes full initialize() and migrations
    {
        let store = RepositoryStore::open(&db_path).unwrap();

        // Verify user_version was bumped to 2
        let v: i32 = store
            .conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, 2, "user_version must be bumped to 2 after migration");

        // Verify attacker repo was demoted, but builtin was kept official
        let attacker = store
            .get_repository_trust_level("attacker-repo")
            .unwrap()
            .unwrap();
        assert_eq!(
            attacker, "community",
            "External repo claiming official must be demoted by v2 migration"
        );
        let builtin = store
            .get_repository_trust_level("builtin-repo")
            .unwrap()
            .unwrap();
        assert_eq!(builtin, "official", "Builtin repo must remain official");

        // Verify valid profile was preserved
        let nes = store
            .get_profile_emulator_config("nes-mesen")
            .unwrap()
            .unwrap();
        assert_eq!(nes.exe_path.as_deref(), Some("C:/Games/Mesen.exe"));
        assert_eq!(nes.version.as_deref(), Some("2.1.1"));

        // Verify normal app_config key was preserved
        assert_eq!(
            store.get_config("download_root").unwrap().as_deref(),
            Some("F:/CustomGames")
        );

        // Verify poisoned records were purged
        let poisoned_switch = store.get_profile_emulator_config("switch-manual").unwrap();
        assert!(
            poisoned_switch.is_none(),
            "Poisoned Switch manifest config must be wiped"
        );

        let poisoned_config = store.get_config("manifest_emulator:poisoned-game").unwrap();
        assert!(
            poisoned_config.is_none(),
            "Poisoned manifest_emulator config must be wiped"
        );

        // 3. User now sets up a new legitimate Switch manifest emulator
        store
            .set_config(
                "manifest_emulator:legit-switch-game",
                r#"{"url":"https://example.com/eden.zip","archive_sha256":"123"}"#,
            )
            .unwrap();
    }

    // 4. Reopen database (simulating restart after migration)
    {
        let store = RepositoryStore::open(&db_path).unwrap();

        // The newly saved Switch manifest emulator must NOT be wiped on restart!
        let legit = store
            .get_config("manifest_emulator:legit-switch-game")
            .unwrap();
        assert!(
            legit.is_some(),
            "Legitimate Switch emulator config must persist across restarts"
        );

        let nes = store
            .get_profile_emulator_config("nes-mesen")
            .unwrap()
            .unwrap();
        assert_eq!(nes.version.as_deref(), Some("2.1.1"));
    }
}
