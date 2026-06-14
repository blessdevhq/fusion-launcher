use std::path::Path;

use tempfile::tempdir;

use super::*;
use crate::schema::{
    AssetKind, InstallHint, InstallTarget, RepositoryAsset, RepositoryGame, RepositoryMetadata,
    RepositorySchema, SourceUri,
};
use crate::storage::RepositoryStore;
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn valid_nes_bytes() -> Vec<u8> {
    let mut bytes = b"NES\x1A".to_vec();
    bytes.extend([0_u8; 32]);
    bytes
}

#[test]
fn open_game_folder_uses_parent_for_game_file() {
    let temp = tempdir().unwrap();
    let game_file = temp.path().join("game.nes");
    std::fs::write(&game_file, valid_nes_bytes()).unwrap();

    assert_eq!(folder_path_for_open(&game_file).unwrap(), temp.path());
}

#[test]
fn open_game_folder_keeps_download_directory() {
    let temp = tempdir().unwrap();
    let game_directory = temp.path().join("game");
    std::fs::create_dir(&game_directory).unwrap();

    assert_eq!(
        folder_path_for_open(&game_directory).unwrap(),
        game_directory
    );
}

fn test_repo(required_asset_ids: Vec<String>) -> RepositorySchema {
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
            display_name: "Demo Emulator".to_string(),
            sources: vec![SourceUri::Http {
                url: "https://example.com/emulator.zip".to_string(),
                sha256: sha256_hex(b"asset"),
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
            required_system_file_ids: required_asset_ids,
            launch: None,
        }],
    }
}

fn user_file_repo(sha256: Option<String>, install_hint: Option<InstallHint>) -> RepositorySchema {
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
            id: "bios".to_string(),
            platform: "nes".to_string(),
            asset_kind: AssetKind::Bios,
            display_name: "Demo BIOS".to_string(),
            sources: vec![SourceUri::UserProvided {
                instructions: Some("Import your dumped BIOS.".to_string()),
                sha256,
                size_bytes: None,
            }],
            install_hint,
            executable: false,
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
            required_system_file_ids: vec!["bios".to_string()],
            launch: None,
        }],
    }
}

fn user_game_repo() -> RepositorySchema {
    RepositorySchema {
        metadata: RepositoryMetadata {
            id: "repo".to_string(),
            name: "Repo".to_string(),
            version: "1".to_string(),
            schema_version: 3,
            maintainer: None,
            homepage_url: None,
            license: None,
            trust_level: None,
            content_hash: None,
            updated_at: None,
        },
        system_files: vec![],
        catalog: vec![RepositoryGame {
            id: "game".to_string(),
            platform: "nes".to_string(),
            title: "Game".to_string(),
            description: None,
            cover_image_url: None,
            trailer_url: None,
            artwork: None,
            metadata: None,
            content_mode: Some("user_provided".to_string()),
            setup_profile_id: Some("future-profile".to_string()),
            downloads: vec![SourceUri::UserProvided {
                instructions: Some("Import your local game file.".to_string()),
                sha256: None,
                size_bytes: None,
            }],
            expected_extensions: vec![".nes".to_string()],
            required_system_file_ids: vec![],
            launch: None,
        }],
    }
}

fn legacy_demo_repo() -> RepositorySchema {
    RepositorySchema {
        metadata: RepositoryMetadata {
            id: "retrohydra-demo".to_string(),
            name: "RetroHydra Official Demo Repository".to_string(),
            version: "1.0.0".to_string(),
            schema_version: 2,
            maintainer: Some("RetroHydra Team".to_string()),
            homepage_url: Some("https://retrohydra.app".to_string()),
            license: None,
            trust_level: Some("official".to_string()),
            content_hash: None,
            updated_at: None,
        },
        system_files: vec![RepositoryAsset {
            id: "ps1_bios_scph1001".to_string(),
            platform: "ps1".to_string(),
            asset_kind: AssetKind::Bios,
            display_name: "PlayStation BIOS".to_string(),
            sources: vec![SourceUri::UserProvided {
                instructions: None,
                sha256: Some("a".repeat(64)),
                size_bytes: None,
            }],
            install_hint: None,
            executable: false,
        }],
        catalog: vec![RepositoryGame {
            id: "nes_http_smoke".to_string(),
            platform: "nes".to_string(),
            title: "RetroHydra NES HTTP Smoke Demo".to_string(),
            description: None,
            cover_image_url: None,
            trailer_url: None,
            artwork: None,
            metadata: None,
            content_mode: None,
            setup_profile_id: None,
            downloads: vec![SourceUri::Http {
                url: "http://localhost:3000/demo-content/retrohydra-demo.nes".to_string(),
                sha256: "b".repeat(64),
                size_bytes: Some(168),
            }],
            expected_extensions: vec![".nes".to_string()],
            required_system_file_ids: vec![],
            launch: None,
        }],
    }
}

fn open_store(required_asset_ids: Vec<String>) -> (tempfile::TempDir, RepositoryStore) {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository(
            "https://example.com/index.json",
            &test_repo(required_asset_ids),
        )
        .unwrap();
    (dir, store)
}

#[test]
fn library_status_marks_installed_game_with_missing_system_file() {
    let (dir, store) = open_store(vec!["emu".to_string()]);
    let game = store.get_game("repo::game").unwrap().unwrap();
    let game_path = dir.path().join("game.nes");
    std::fs::write(&game_path, valid_nes_bytes()).unwrap();
    let game_path_string = game_path.to_string_lossy().to_string();
    store
        .record_download("repo::game", "game", Some(&game_path_string), None, None)
        .unwrap();

    let status = build_library_status(&store, dir.path(), &game).unwrap();

    assert!(status.installed);
    assert!(!status.system_requirements_ready);
    assert_eq!(
        status.missing_requirements,
        vec!["Demo Emulator is not installed".to_string()]
    );
}

#[test]
fn library_status_marks_trusted_downloaded_requirements_ready() {
    let (dir, store) = open_store(vec!["emu".to_string()]);
    let game = store.get_game("repo::game").unwrap().unwrap();
    let game_path = dir.path().join("game.nes");
    let asset_path = dir.path().join("System").join("emu.exe");
    std::fs::create_dir_all(asset_path.parent().unwrap()).unwrap();
    std::fs::write(&game_path, valid_nes_bytes()).unwrap();
    std::fs::write(&asset_path, b"asset").unwrap();
    let game_path_string = game_path.to_string_lossy().to_string();
    let asset_path_string = asset_path.to_string_lossy().to_string();
    let asset_sha = sha256_hex(b"asset");
    store
        .record_download("repo::game", "game", Some(&game_path_string), None, None)
        .unwrap();
    store
        .record_download(
            "repo::emu",
            "asset",
            Some(&asset_path_string),
            Some(&asset_sha),
            None,
        )
        .unwrap();
    store
        .trust_executable("repo::emu", &asset_path_string, &asset_sha)
        .unwrap();

    let status = build_library_status(&store, dir.path(), &game).unwrap();

    assert!(status.installed);
    assert!(status.system_requirements_ready);
    assert!(status.missing_requirements.is_empty());
}

#[test]
fn imports_user_file_and_records_completed_download() {
    let dir = tempdir().unwrap();
    let asset_sha = sha256_hex(b"bios");
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository(
            "https://example.com/index.json",
            &user_file_repo(
                Some(asset_sha.clone()),
                Some(InstallHint {
                    target: InstallTarget::AppSystem,
                    relative_path: Some("bios/demo.bin".to_string()),
                }),
            ),
        )
        .unwrap();
    let source_path = dir.path().join("source.bin");
    std::fs::write(&source_path, b"bios").unwrap();

    let report = import_asset_file_into_store(&store, dir.path(), "repo::bios", &source_path);

    assert_eq!(report.status, "installed");
    assert!(report.error_code.is_none());
    assert_eq!(
        std::fs::read(&report.installed_path).unwrap(),
        b"bios".to_vec()
    );
    let download = store.get_download("repo::bios").unwrap().unwrap();
    assert_eq!(download.status, "completed");
    assert_eq!(download.source, "user_import");
    assert_eq!(download.magnet_uri, "");
    assert_eq!(download.sha256.as_deref(), Some(asset_sha.as_str()));

    let game = store.get_game("repo::game").unwrap().unwrap();
    let requirements = build_requirements_report(&store, dir.path(), &game).unwrap();
    assert_eq!(
        requirements.requirements[0].checksum.as_deref(),
        Some(asset_sha.as_str())
    );
    assert!(requirements.requirements[0].downloaded);
}

#[test]
fn imports_user_game_file_and_records_completed_download() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository("https://example.com/index.json", &user_game_repo())
        .unwrap();
    let source_path = dir.path().join("source.nes");
    let bytes = valid_nes_bytes();
    std::fs::write(&source_path, &bytes).unwrap();

    let report = import_game_file_into_store(
        &store,
        &dir.path().join("Games"),
        "repo::game",
        &source_path,
    );

    assert_eq!(report.status, "installed");
    assert!(report.error_code.is_none());
    assert_eq!(report.sha256.as_deref(), Some(sha256_hex(&bytes).as_str()));
    let download = store.get_download("repo::game").unwrap().unwrap();
    assert_eq!(download.status, "ready");
    assert_eq!(download.source, "legacy");
    let torrent = store.get_torrent_download("repo::game").unwrap().unwrap();
    assert_eq!(torrent.status, "completed");
    assert_eq!(torrent.magnet_uri, "direct:user_import");

    let game = store.get_game("repo::game").unwrap().unwrap();
    let status = build_library_status(&store, dir.path(), &game).unwrap();
    assert!(status.installed);
    assert!(status.system_requirements_ready);
}

#[test]
fn user_game_import_rejects_wrong_extension() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository("https://example.com/index.json", &user_game_repo())
        .unwrap();
    let source_path = dir.path().join("source.txt");
    std::fs::write(&source_path, b"not a rom").unwrap();

    let report = import_game_file_into_store(
        &store,
        &dir.path().join("Games"),
        "repo::game",
        &source_path,
    );

    assert_eq!(report.status, "error");
    assert_eq!(report.error_code.as_deref(), Some("wrong_extension"));
    assert!(store.get_download("repo::game").unwrap().is_none());
}

#[test]
fn existing_matching_user_file_returns_already_installed_without_copy() {
    let dir = tempdir().unwrap();
    let asset_sha = sha256_hex(b"bios");
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository(
            "https://example.com/index.json",
            &user_file_repo(
                Some(asset_sha.clone()),
                Some(InstallHint {
                    target: InstallTarget::AppSystem,
                    relative_path: Some("bios/demo.bin".to_string()),
                }),
            ),
        )
        .unwrap();
    let target = dir
        .path()
        .join("System")
        .join("nes")
        .join("bios")
        .join("demo.bin");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"bios").unwrap();
    let missing_source = dir.path().join("missing.bin");

    let report = import_asset_file_into_store(&store, dir.path(), "repo::bios", &missing_source);

    assert_eq!(report.status, "already_installed");
    assert_eq!(std::fs::read(&target).unwrap(), b"bios".to_vec());
    let download = store.get_download("repo::bios").unwrap().unwrap();
    assert_eq!(download.status, "completed");
    assert_eq!(download.source, "user_import");
}

#[test]
fn user_file_checksum_mismatch_returns_error_without_copy() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository(
            "https://example.com/index.json",
            &user_file_repo(
                Some(sha256_hex(b"expected")),
                Some(InstallHint {
                    target: InstallTarget::AppSystem,
                    relative_path: Some("bios/demo.bin".to_string()),
                }),
            ),
        )
        .unwrap();
    let source_path = dir.path().join("bad-source.bin");
    std::fs::write(&source_path, b"wrong").unwrap();

    let report = import_asset_file_into_store(&store, dir.path(), "repo::bios", &source_path);

    assert_eq!(report.status, "error");
    assert_eq!(report.error_code.as_deref(), Some("checksum_mismatch"));
    assert!(!Path::new(&report.installed_path).exists());
    assert!(store.get_download("repo::bios").unwrap().is_none());
}

#[test]
fn user_file_import_reports_stable_error_codes() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository(
            "https://example.com/index.json",
            &user_file_repo(
                None,
                Some(InstallHint {
                    target: InstallTarget::AppSystem,
                    relative_path: Some("bios/demo.bin".to_string()),
                }),
            ),
        )
        .unwrap();

    let unknown = import_asset_file_into_store(
        &store,
        dir.path(),
        "repo::missing",
        &dir.path().join("source.bin"),
    );
    assert_eq!(unknown.error_code.as_deref(), Some("unknown_asset"));

    let missing = import_asset_file_into_store(
        &store,
        dir.path(),
        "repo::bios",
        &dir.path().join("source.bin"),
    );
    assert_eq!(missing.error_code.as_deref(), Some("source_missing"));

    let source_dir = dir.path().join("source-dir");
    std::fs::create_dir(&source_dir).unwrap();
    let directory = import_asset_file_into_store(&store, dir.path(), "repo::bios", &source_dir);
    assert_eq!(directory.error_code.as_deref(), Some("source_not_file"));

    let mut unsupported_store = RepositoryStore::open(&dir.path().join("unsupported.db")).unwrap();
    unsupported_store
        .store_repository(
            "https://example.com/unsupported.json",
            &user_file_repo(
                None,
                Some(InstallHint {
                    target: InstallTarget::UserSelected,
                    relative_path: None,
                }),
            ),
        )
        .unwrap();
    let source_path = dir.path().join("source.bin");
    std::fs::write(&source_path, b"bios").unwrap();
    let unsupported =
        import_asset_file_into_store(&unsupported_store, dir.path(), "repo::bios", &source_path);
    assert_eq!(
        unsupported.error_code.as_deref(),
        Some("unsupported_target")
    );
}

#[test]
fn completed_torrent_download_counts_as_installed_through_legacy_record() {
    let (dir, store) = open_store(Vec::new());
    let game = store.get_game("repo::game").unwrap().unwrap();
    let game_dir = dir.path().join("downloaded-game");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::write(game_dir.join("game.nes"), valid_nes_bytes()).unwrap();
    let game_dir_string = game_dir.to_string_lossy().to_string();
    store
        .upsert_torrent_download_start("repo::game", "magnet:?xt=urn:btih:abc", &game_dir_string)
        .unwrap();
    store
        .mark_torrent_completed("repo::game", &game_dir_string, 100, 100)
        .unwrap();

    let status = build_library_status(&store, dir.path(), &game).unwrap();

    assert!(status.installed);
    assert!(status.system_requirements_ready);
    assert_eq!(
        status
            .download
            .as_ref()
            .map(|download| download.status.as_str()),
        Some("completed")
    );
}

#[test]
fn corrupt_game_file_blocks_ready_status() {
    let (dir, store) = open_store(Vec::new());
    let game = store.get_game("repo::game").unwrap().unwrap();
    let game_path = dir.path().join("bad.nes");
    std::fs::write(&game_path, b"not an ines file").unwrap();
    let game_path_string = game_path.to_string_lossy().to_string();
    store
        .record_download("repo::game", "game", Some(&game_path_string), None, None)
        .unwrap();

    let status = build_library_status(&store, dir.path(), &game).unwrap();

    assert!(status.installed);
    assert!(!status.system_requirements_ready);
    assert!(status
        .missing_requirements
        .iter()
        .any(|message| message.starts_with("Game file:")));
}

#[test]
fn repairs_legacy_localhost_demo_repository() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository(
            "http://localhost:3000/demo-repository.json",
            &legacy_demo_repo(),
        )
        .unwrap();
    let stale_path = dir
        .path()
        .join("Games")
        .join("nes")
        .join("retrohydra-demo--nes_http_smoke")
        .join("retrohydra-demo.nes");
    std::fs::create_dir_all(stale_path.parent().unwrap()).unwrap();
    std::fs::write(&stale_path, b"not an ines file").unwrap();
    let stale_path_string = stale_path.to_string_lossy().to_string();
    store
        .record_download(
            "retrohydra-demo::nes_http_smoke",
            "game",
            Some(&stale_path_string),
            None,
            None,
        )
        .unwrap();

    let report = repair_library_state(&mut store, dir.path()).unwrap();

    assert!(report.repaired);
    assert!(!stale_path.exists());
    assert!(store
        .get_game("retrohydra-demo::nes_http_smoke")
        .unwrap()
        .is_none());
    assert!(store
        .get_game("fusion-launcher-demo::fusion_launcher_nes_smoke")
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .get_repository_url("fusion-launcher-demo")
            .unwrap()
            .as_deref(),
        Some(crate::builtin_demo::BUILTIN_DEMO_REPOSITORY_URL)
    );
    assert!(store
        .get_repository_url("retrohydra-demo")
        .unwrap()
        .is_none());
    assert!(store
        .get_download("retrohydra-demo::nes_http_smoke")
        .unwrap()
        .is_none());
}

#[test]
fn loads_repository_from_local_json_file() {
    let dir = tempdir().unwrap();
    let repo_path = dir.path().join("repository.json");
    std::fs::write(
        &repo_path,
        serde_json::to_string(&test_repo(Vec::new())).unwrap(),
    )
    .unwrap();

    let (repo, url) = load_repository_schema_from_file(&repo_path.to_string_lossy()).unwrap();

    assert_eq!(repo.metadata.id, "repo");
    assert!(url.starts_with("file:"));
}
