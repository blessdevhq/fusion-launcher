use std::fs::File;
use std::io::Write;
use std::path::Path;

use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::*;
use crate::schema::{
    AssetKind, AssetView, InstallHint, InstallTarget, RepositoryAsset, RepositoryGame,
    RepositoryMetadata, RepositorySchema, SourceUri,
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

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    for (name, bytes) in entries {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn manifest_emulator_asset_view() -> AssetView {
    AssetView {
        id: "manifest-demo::0100-emulator-bundle".to_string(),
        source_id: "0100-emulator-bundle".to_string(),
        repository_id: "manifest-demo".to_string(),
        platform: "switch".to_string(),
        asset_kind: AssetKind::Emulator,
        display_name: "Switch emulator bundle".to_string(),
        sources: vec![SourceUri::Http {
            url: "https://example.com/eden.zip".to_string(),
            sha256: "a".repeat(64),
            size_bytes: None,
        }],
        install_hint: None,
        executable: false,
    }
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

#[test]
fn manifest_emulator_zip_extracts_into_parent_and_deletes_archive() {
    let temp = tempdir().unwrap();
    let bundle_dir = temp.path().join("System").join("switch").join("bundle");
    std::fs::create_dir_all(&bundle_dir).unwrap();
    let archive_path = bundle_dir.join("eden.zip");
    write_zip(&archive_path, &[("eden.exe", b"exe")]);

    let extracted =
        extract_manifest_emulator_bundle_archive(&manifest_emulator_asset_view(), &archive_path)
            .unwrap();

    assert_eq!(extracted, bundle_dir);
    assert!(extracted.join("eden.exe").is_file());
    assert!(!archive_path.exists());
}

#[test]
fn manifest_emulator_zip_rejects_path_traversal_without_deleting_archive() {
    let temp = tempdir().unwrap();
    let bundle_dir = temp.path().join("System").join("switch").join("bundle");
    std::fs::create_dir_all(&bundle_dir).unwrap();
    let archive_path = bundle_dir.join("eden.zip");
    write_zip(&archive_path, &[("../outside.exe", b"bad")]);

    let error =
        extract_manifest_emulator_bundle_archive(&manifest_emulator_asset_view(), &archive_path)
            .unwrap_err();

    assert!(error.contains("path traversal"));
    assert!(archive_path.exists());
    assert!(!temp.path().join("outside.exe").exists());
}

#[test]
fn default_asset_targets_split_system_and_manifest_emulators() {
    let temp = tempdir().unwrap();
    let store = RepositoryStore::open(&temp.path().join("fusion-launcher.db")).unwrap();
    let emulator = manifest_emulator_asset_view();
    let source = emulator.sources.first().unwrap();
    let emulator_target = resolve_asset_target(&store, temp.path(), &emulator, source).unwrap();

    assert_eq!(
        emulator_target,
        temp.path()
            .join("Emulators")
            .join("switch")
            .join("manifest-demo--0100-emulator-bundle")
            .join("eden.zip")
    );

    let mut system_asset = emulator.clone();
    system_asset.id = "repo::prod-keys".to_string();
    system_asset.source_id = "prod-keys".to_string();
    system_asset.repository_id = "repo".to_string();
    system_asset.asset_kind = AssetKind::Keys;
    system_asset.display_name = "prod.keys".to_string();
    system_asset.sources = vec![SourceUri::Http {
        url: "https://example.com/prod.keys".to_string(),
        sha256: "b".repeat(64),
        size_bytes: None,
    }];
    let system_source = system_asset.sources.first().unwrap();
    let system_target =
        resolve_asset_target(&store, temp.path(), &system_asset, system_source).unwrap();

    assert_eq!(
        system_target,
        temp.path()
            .join("System")
            .join("switch")
            .join("repo--prod-keys")
            .join("prod.keys")
    );
}

#[test]
fn manifest_emulator_target_override_uses_asset_subfolder() {
    let temp = tempdir().unwrap();
    let store = RepositoryStore::open(&temp.path().join("fusion-launcher.db")).unwrap();
    let emulator = manifest_emulator_asset_view();
    let source = emulator.sources.first().unwrap();
    let selected = temp.path().join("Selected");

    let target =
        resolve_asset_target_with_override(&store, temp.path(), &emulator, source, Some(&selected))
            .unwrap();

    assert_eq!(
        target,
        selected
            .join("manifest-demo--0100-emulator-bundle")
            .join("eden.zip")
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

fn manifest_emulator_repo(archive_sha: String) -> RepositorySchema {
    RepositorySchema {
        metadata: RepositoryMetadata {
            id: "manifest-demo".to_string(),
            name: "Manifest Demo".to_string(),
            version: "1".to_string(),
            schema_version: 1,
            maintainer: None,
            homepage_url: None,
            license: None,
            trust_level: Some("unknown".to_string()),
            content_hash: None,
            updated_at: None,
        },
        system_files: vec![RepositoryAsset {
            id: "0100-emulator-bundle".to_string(),
            platform: "switch".to_string(),
            asset_kind: AssetKind::Emulator,
            display_name: "Switch emulator bundle".to_string(),
            sources: vec![SourceUri::Http {
                url: "https://example.com/eden.zip".to_string(),
                sha256: archive_sha,
                size_bytes: None,
            }],
            install_hint: None,
            executable: false,
        }],
        catalog: vec![RepositoryGame {
            id: "game".to_string(),
            platform: "switch".to_string(),
            title: "Switch Game".to_string(),
            description: None,
            cover_image_url: None,
            trailer_url: None,
            artwork: None,
            metadata: None,
            content_mode: Some("user_provided".to_string()),
            setup_profile_id: None,
            downloads: vec![SourceUri::UserProvided {
                instructions: None,
                sha256: None,
                size_bytes: None,
            }],
            expected_extensions: vec![".nsp".to_string()],
            required_system_file_ids: vec!["0100-emulator-bundle".to_string()],
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
fn removing_downloadable_profile_deletes_managed_folder_and_config() {
    let (dir, store) = open_store(Vec::new());
    let emulator_dir = dir.path().join("Emulators").join("nes").join("nes-mesen");
    let exe_path = emulator_dir.join("Mesen.exe");
    std::fs::create_dir_all(&emulator_dir).unwrap();
    std::fs::write(&exe_path, b"exe").unwrap();
    let exe_path = exe_path.to_string_lossy().to_string();
    store
        .upsert_profile_emulator_config(
            "nes-mesen",
            "nes",
            Some(&exe_path),
            "valid",
            Some("2.1.1"),
            Some("{game_path}"),
        )
        .unwrap();

    let report = remove_profile_emulator_from_store(&store, dir.path(), "nes-mesen").unwrap();

    assert_eq!(report.profile_id, "nes-mesen");
    assert_eq!(report.platform, "nes");
    assert!(report.deleted_files);
    assert!(report.removed_config);
    assert!(!emulator_dir.exists());
    assert!(store
        .get_profile_emulator_config("nes-mesen")
        .unwrap()
        .is_none());
}

#[test]
fn removing_downloadable_profile_handles_missing_managed_folder() {
    let (dir, store) = open_store(Vec::new());
    store
        .upsert_profile_emulator_config(
            "nes-mesen",
            "nes",
            Some("C:/Missing/Mesen.exe"),
            "missing",
            None,
            Some("{game_path}"),
        )
        .unwrap();

    let report = remove_profile_emulator_from_store(&store, dir.path(), "nes-mesen").unwrap();

    assert!(!report.deleted_files);
    assert!(report.removed_config);
    assert!(store
        .get_profile_emulator_config("nes-mesen")
        .unwrap()
        .is_none());
}

#[test]
fn removing_manual_profile_only_forgets_external_path() {
    let (dir, store) = open_store(Vec::new());
    let external_dir = dir.path().join("External");
    let external_exe = external_dir.join("eden.exe");
    std::fs::create_dir_all(&external_dir).unwrap();
    std::fs::write(&external_exe, b"exe").unwrap();
    let external_exe = external_exe.to_string_lossy().to_string();
    store
        .upsert_profile_emulator_config(
            "switch-manual",
            "switch",
            Some(&external_exe),
            "valid",
            None,
            Some("{game_path}"),
        )
        .unwrap();

    let report = remove_profile_emulator_from_store(&store, dir.path(), "switch-manual").unwrap();

    assert!(!report.deleted_files);
    assert!(report.removed_path.is_none());
    assert!(Path::new(&external_exe).exists());
    assert!(store
        .get_profile_emulator_config("switch-manual")
        .unwrap()
        .is_none());
}

#[test]
fn adopts_switch_keys_bundled_in_emulator_archive() {
    let (dir, store) = open_store(Vec::new());
    let profile = crate::setup_profiles::get_platform_setup_profile("switch-manual").unwrap();
    let keys_req = profile
        .system_files
        .iter()
        .find(|requirement| requirement.id == "switch-prod-keys")
        .unwrap();

    // Simulate an extracted emulator that ships prod.keys in a nested folder.
    let search_dir = dir.path().join("Emulators").join("switch");
    let nested = search_dir.join("keys");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("prod.keys"), b"AAAA").unwrap();

    let adopted =
        adopt_bundled_profile_system_files(&store, dir.path(), dir.path(), &profile, &search_dir)
            .unwrap();
    assert_eq!(adopted, vec!["switch-prod-keys".to_string()]);

    let state = inspect_profile_system_file(&store, dir.path(), &profile, keys_req).unwrap();
    assert_eq!(state.status, "ready");
}

#[test]
fn leaves_switch_keys_unsatisfied_when_archive_lacks_them() {
    let (dir, store) = open_store(Vec::new());
    let profile = crate::setup_profiles::get_platform_setup_profile("switch-manual").unwrap();
    let keys_req = profile
        .system_files
        .iter()
        .find(|requirement| requirement.id == "switch-prod-keys")
        .unwrap();

    let search_dir = dir.path().join("Emulators").join("switch");
    std::fs::create_dir_all(&search_dir).unwrap();

    let adopted =
        adopt_bundled_profile_system_files(&store, dir.path(), dir.path(), &profile, &search_dir)
            .unwrap();
    assert!(adopted.is_empty());

    let state = inspect_profile_system_file(&store, dir.path(), &profile, keys_req).unwrap();
    assert_eq!(state.status, "missing");
}

#[test]
fn manifest_emulator_bundle_directory_satisfies_requirement_after_zip_delete() {
    let dir = tempdir().unwrap();
    let archive_sha = "b".repeat(64);
    let mut store = RepositoryStore::open(&dir.path().join("fusion-launcher.db")).unwrap();
    store
        .store_repository(
            "manifest:inline:manifest-demo",
            &manifest_emulator_repo(archive_sha.clone()),
        )
        .unwrap();

    let asset_id = "manifest-demo::0100-emulator-bundle";
    let bundle_dir = dir
        .path()
        .join("Emulators")
        .join("switch")
        .join("manifest-demo--0100-emulator-bundle");
    std::fs::create_dir_all(&bundle_dir).unwrap();
    std::fs::write(bundle_dir.join("eden.exe"), b"exe").unwrap();
    let bundle_dir_string = bundle_dir.to_string_lossy().to_string();
    store
        .record_download(
            asset_id,
            "asset",
            Some(&bundle_dir_string),
            Some(&archive_sha),
            None,
        )
        .unwrap();

    let game = store.get_game("manifest-demo::game").unwrap().unwrap();
    let requirements = build_requirements_report(&store, dir.path(), &game).unwrap();

    assert_eq!(requirements.requirements.len(), 1);
    let requirement = &requirements.requirements[0];
    assert!(requirement.downloaded);
    assert_eq!(requirement.status, "ready");
    assert_eq!(
        requirement.local_path.as_deref(),
        Some(bundle_dir_string.as_str())
    );
    assert_eq!(
        requirement.target_path.as_deref(),
        Some(bundle_dir_string.as_str())
    );
    assert_eq!(requirement.checksum.as_deref(), Some(archive_sha.as_str()));
    assert_eq!(requirement.sha256.as_deref(), Some(archive_sha.as_str()));
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
