use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};

use crate::downloads::{
    destination_for_source, download_source_to_file, file_name_for_source, hash_file,
};
use crate::logging;
use crate::platform::{default_launch_args_template, is_mvp_platform, MVP_PLATFORM_CONFIGS};
use crate::schema::{
    AssetView, CatalogGameView, DiagnosticsBundle, DownloadRecord, GameDownloadStartReport,
    HealthCheckItem, HealthReport, EmulatorConfig, InstallTarget, LibraryGameStatus,
    OnboardingState, RepositoryPreview, RepositorySchema, RepositorySummary, RequirementItem,
    RequirementsReport, SourceUri, TrustedExecutable,
};
use crate::security::{validate_platform, validate_repository_schema, validate_repository_url};
use crate::storage::RepositoryStore;
use crate::AppState;

#[tauri::command]
pub async fn preview_repository(
    url: String,
    state: State<'_, AppState>,
) -> Result<RepositoryPreview, String> {
    let allow_dev_http = cfg!(debug_assertions);
    let repo = fetch_repository_schema(&url, allow_dev_http).await?;
    let preview = build_repository_preview(&url, &repo);
    logging::log_event(
        &state.data_dir,
        "repository_previewed",
        &[("url", url.as_str()), ("repository_id", preview.id.as_str())],
    );
    Ok(preview)
}

#[tauri::command]
pub async fn connect_repository(
    url: String,
    state: State<'_, AppState>,
) -> Result<RepositorySummary, String> {
    let allow_dev_http = cfg!(debug_assertions);
    let repo = fetch_repository_schema(&url, allow_dev_http).await?;

    let mut store = lock_store(&state)?;
    let summary = store.store_repository(&url, &repo)?;
    logging::log_event(
        &state.data_dir,
        "repository_connected",
        &[("url", url.as_str()), ("repository_id", summary.id.as_str())],
    );
    Ok(summary)
}

#[tauri::command]
pub async fn refresh_repository(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<RepositorySummary, String> {
    let url = {
        let store = lock_store(&state)?;
        store
            .get_repository_url(&repository_id)?
            .ok_or_else(|| format!("Unknown repository: {repository_id}"))?
    };
    let allow_dev_http = cfg!(debug_assertions);
    let repo = fetch_repository_schema(&url, allow_dev_http).await?;
    let mut store = lock_store(&state)?;
    let summary = store.store_repository(&url, &repo)?;
    logging::log_event(
        &state.data_dir,
        "repository_refreshed",
        &[("url", url.as_str()), ("repository_id", summary.id.as_str())],
    );
    Ok(summary)
}

#[tauri::command]
pub fn list_repositories(state: State<'_, AppState>) -> Result<Vec<RepositorySummary>, String> {
    lock_store(&state)?.list_repositories()
}

#[tauri::command]
pub fn get_onboarding_state(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    let store = lock_store(&state)?;
    let repositories = store.list_repositories()?;
    let catalog_count = store.get_catalog()?.len();
    let valid_emulator_count = store
        .list_emulator_configs()?
        .into_iter()
        .filter(|config| is_mvp_platform(&config.platform))
        .filter(|config| config.status == "valid")
        .count();
    let repositories_configured = !repositories.is_empty();
    let emulators_configured = valid_emulator_count > 0;
    let step = if !repositories_configured {
        "addRepository"
    } else if !emulators_configured {
        "configureEmulator"
    } else {
        "complete"
    };

    Ok(OnboardingState {
        step: step.to_string(),
        repositories_configured,
        emulators_configured,
        catalog_count,
        valid_emulator_count,
    })
}

#[tauri::command]
pub fn disconnect_repository(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    lock_store(&state)?.disconnect_repository(&repository_id)
}

#[tauri::command]
pub fn get_catalog(state: State<'_, AppState>) -> Result<Vec<CatalogGameView>, String> {
    lock_store(&state)?.get_catalog()
}

#[tauri::command]
pub fn get_game(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Option<CatalogGameView>, String> {
    lock_store(&state)?.get_game(&game_id)
}

#[tauri::command]
pub fn check_requirements(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<RequirementsReport, String> {
    let store = lock_store(&state)?;
    let game = store
        .get_game(&game_id)?
        .ok_or_else(|| format!("Unknown game: {game_id}"))?;
    build_requirements_report(&store, &state.data_dir, &game)
}

#[tauri::command]
pub fn get_library_statuses(state: State<'_, AppState>) -> Result<Vec<LibraryGameStatus>, String> {
    let store = lock_store(&state)?;
    store
        .get_catalog()?
        .iter()
        .map(|game| build_library_status(&store, &state.data_dir, game))
        .collect()
}

#[tauri::command]
pub fn list_emulator_configs(state: State<'_, AppState>) -> Result<Vec<EmulatorConfig>, String> {
    let store = lock_store(&state)?;
    let configs = store.list_emulator_configs()?;

    configs
        .into_iter()
        .map(|config| {
            let status = validate_emulator_status(config.exe_path.as_deref());
            store.upsert_emulator_config(
                &config.platform,
                config.exe_path.as_deref(),
                status,
                config.version.as_deref(),
                config.launch_args_template.as_deref(),
            )
        })
        .collect()
}

#[tauri::command]
pub fn save_emulator_config(
    platform: String,
    exe_path: String,
    launch_args_template: Option<String>,
    state: State<'_, AppState>,
) -> Result<EmulatorConfig, String> {
    validate_platform(&platform)?;
    let normalized_path = exe_path.trim();
    let status = validate_emulator_status(Some(normalized_path));
    let template = normalize_launch_args_template(launch_args_template)
        .or_else(|| default_launch_args_template(&platform).map(ToString::to_string));

    lock_store(&state)?.upsert_emulator_config(
        &platform,
        Some(normalized_path),
        status,
        None,
        template.as_deref(),
    )
}

#[tauri::command]
pub fn validate_emulator_config(
    platform: String,
    state: State<'_, AppState>,
) -> Result<EmulatorConfig, String> {
    validate_platform(&platform)?;
    let store = lock_store(&state)?;
    let config = store
        .get_emulator_config(&platform)?
        .ok_or_else(|| format!("No emulator config is stored for {platform}"))?;
    let status = validate_emulator_status(config.exe_path.as_deref());

    store.upsert_emulator_config(
        &platform,
        config.exe_path.as_deref(),
        status,
        config.version.as_deref(),
        config.launch_args_template.as_deref(),
    )
}

#[tauri::command]
pub fn delete_emulator_config(
    platform: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    validate_platform(&platform)?;
    lock_store(&state)?.delete_emulator_config(&platform)
}

#[tauri::command]
pub async fn download_asset(
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<DownloadRecord, String> {
    let asset = {
        let store = lock_store(&state)?;
        store
            .get_asset(&asset_id)?
            .ok_or_else(|| format!("Unknown asset: {asset_id}"))?
    };

    let source = asset
        .sources
        .first()
        .ok_or_else(|| format!("Asset {} has no sources.", asset.display_name))?;
    if matches!(source, SourceUri::UserProvided { .. }) {
        let destination = {
            let store = lock_store(&state)?;
            resolve_asset_target(&store, &state.data_dir, &asset, source)
                .map_err(|blocked| blocked.message)?
        };
        let target_path = destination.to_string_lossy().to_string();
        let message = format!(
            "{} is user-provided. Place the file at {} or use Import file.",
            asset.display_name, target_path
        );
        let store = lock_store(&state)?;
        let _ = store.record_asset_installation(
            &asset.id,
            Some(&target_path),
            "missing",
            None,
            Some(&message),
        );
        let _ = store.record_download(&asset.id, "asset", None, None, Some(&message));
        return Err(message);
    }
    let destination = {
        let store = lock_store(&state)?;
        match resolve_asset_target(&store, &state.data_dir, &asset, source) {
            Ok(path) => path,
            Err(blocked) => {
                let _ = store.record_asset_installation(
                    &asset.id,
                    blocked.target_path.as_deref(),
                    "blocked",
                    None,
                    Some(&blocked.message),
                );
                let _ =
                    store.record_download(&asset.id, "asset", None, None, Some(&blocked.message));
                return Err(blocked.message);
            }
        }
    };

    match download_source_to_file(source, &destination).await {
        Ok(file) => {
            let local_path = file.path.to_string_lossy().to_string();
            let store = lock_store(&state)?;
            store.record_asset_installation(
                &asset.id,
                Some(&local_path),
                "ready",
                Some(&file.sha256),
                None,
            )?;
            store.record_download(
                &asset.id,
                "asset",
                Some(&local_path),
                Some(&file.sha256),
                None,
            )
        }
        Err(error) => {
            let store = lock_store(&state)?;
            let target_path = destination.to_string_lossy().to_string();
            let _ = store.record_asset_installation(
                &asset.id,
                Some(&target_path),
                "error",
                None,
                Some(&error),
            );
            let _ = store.record_download(&asset.id, "asset", None, None, Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn download_game(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<DownloadRecord, String> {
    let game = {
        let store = lock_store(&state)?;
        store
            .get_game(&game_id)?
            .ok_or_else(|| format!("Unknown game: {game_id}"))?
    };

    let source = game
        .downloads
        .first()
        .ok_or_else(|| format!("Game {} has no download sources.", game.title))?;
    let destination = destination_for_source(
        &state.data_dir.join("Games"),
        &game.platform,
        &game.id,
        source,
        &game.title,
    );

    match download_source_to_file(source, &destination).await {
        Ok(file) => {
            let local_path = file.path.to_string_lossy().to_string();
            lock_store(&state)?.record_download(
                &game.id,
                "game",
                Some(&local_path),
                Some(&file.sha256),
                None,
            )
        }
        Err(error) => {
            let _ = lock_store(&state)?.record_download(&game.id, "game", None, None, Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn start_game_download(
    game_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<GameDownloadStartReport, String> {
    let game = {
        let store = lock_store(&state)?;
        store
            .get_game(&game_id)?
            .ok_or_else(|| format!("Unknown game: {game_id}"))?
    };
    let source = game
        .downloads
        .first()
        .ok_or_else(|| format!("Game {} has no download sources.", game.title))?;
    let download_root = download_root(&state)?;

    match source {
        SourceUri::Magnet {
            uri, size_bytes, ..
        } => {
            preflight_disk_space(&download_root, *size_bytes)?;
            let save_dir = download_root
                .join(crate::downloads::safe_segment(&game.platform))
                .join(crate::downloads::safe_segment(&game.id));
            let save_dir_string = save_dir.to_string_lossy().to_string();
            let torrent = state
                .torrents
                .start_magnet_download(game.id.clone(), uri.clone(), save_dir_string.clone())
                .await?;
            logging::log_event(
                &state.data_dir,
                "game_download_started",
                &[("game_id", game.id.as_str()), ("source", "magnet")],
            );
            Ok(GameDownloadStartReport {
                game_id: game.id,
                source_kind: "magnet".to_string(),
                save_dir: torrent.save_dir.clone(),
                record: None,
                torrent: state.torrents.get_game_download(&game_id)?,
            })
        }
        SourceUri::Http { size_bytes, .. } => {
            preflight_disk_space(&download_root, *size_bytes)?;
            let destination = destination_for_source(
                &download_root,
                &game.platform,
                &game.id,
                source,
                &game.title,
            );
            let file = download_source_to_file(source, &destination).await?;
            let local_path = file.path.to_string_lossy().to_string();
            let record = lock_store(&state)?.record_download(
                &game.id,
                "game",
                Some(&local_path),
                Some(&file.sha256),
                None,
            )?;
            emit_http_download_completed(&app, &game.id, &local_path, size_bytes.unwrap_or(0))?;
            logging::log_event(
                &state.data_dir,
                "game_download_completed",
                &[("game_id", game.id.as_str()), ("source", "http")],
            );
            Ok(GameDownloadStartReport {
                game_id: game.id,
                source_kind: "http".to_string(),
                save_dir: local_path,
                record: Some(record),
                torrent: None,
            })
        }
        SourceUri::UserProvided { .. } => Err("Game downloads cannot be user-provided.".to_string()),
    }
}

#[tauri::command]
pub fn trust_executable(
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<TrustedExecutable, String> {
    let store = lock_store(&state)?;
    let asset = store
        .get_asset(&asset_id)?
        .ok_or_else(|| format!("Unknown asset: {asset_id}"))?;
    if !asset.executable {
        return Err(format!(
            "Asset {} is not marked executable.",
            asset.display_name
        ));
    }

    let download = store.get_download(&asset.id)?.ok_or_else(|| {
        format!(
            "Executable asset {} has not been downloaded.",
            asset.display_name
        )
    })?;
    if download.status != "ready" {
        return Err(format!(
            "Executable asset {} is not ready.",
            asset.display_name
        ));
    }

    let local_path = download
        .local_path
        .ok_or_else(|| format!("Executable asset {} has no local path.", asset.display_name))?;
    let sha256 = download.sha256.ok_or_else(|| {
        format!(
            "Executable asset {} has no verified SHA-256.",
            asset.display_name
        )
    })?;
    if !Path::new(&local_path).exists() {
        return Err(format!("Executable file is missing: {local_path}"));
    }

    store.trust_executable(&asset.id, &local_path, &sha256)
}

#[tauri::command]
pub fn get_download_root(state: State<'_, AppState>) -> Result<String, String> {
    Ok(download_root(&state)?.to_string_lossy().to_string())
}

#[tauri::command]
pub fn set_download_root(path: String, state: State<'_, AppState>) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Download folder cannot be empty.".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("Download folder must be an absolute path.".to_string());
    }
    fs::create_dir_all(&path)
        .map_err(|error| format!("Failed to create download folder: {error}"))?;
    let value = path.to_string_lossy().to_string();
    lock_store(&state)?.set_config("download_root", &value)?;
    logging::log_event(
        &state.data_dir,
        "download_root_changed",
        &[("path", value.as_str())],
    );
    Ok(value)
}

#[tauri::command]
pub async fn remove_game(
    game_id: String,
    delete_files: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let download = {
        let store = lock_store(&state)?;
        (
            store.get_download(&game_id)?,
            store.get_torrent_download(&game_id)?,
        )
    };

    if let Some(torrent) = download.1.as_ref() {
        if !matches!(torrent.status.as_str(), "completed" | "cancelled") {
            let _ = state.torrents.cancel_download(game_id.clone()).await;
        }
    }

    if delete_files {
        let mut candidates = Vec::new();
        if let Some(record) = download.0.as_ref() {
            if let Some(path) = record.local_path.as_ref() {
                candidates.push(PathBuf::from(path));
            }
        }
        if let Some(torrent) = download.1.as_ref() {
            candidates.push(PathBuf::from(&torrent.save_dir));
        }
        for candidate in candidates {
            remove_path_if_allowed(&state.data_dir, &download_root(&state)?, &candidate)?;
        }
    }

    let store = lock_store(&state)?;
    let mut changed = store.delete_download(&game_id)?;
    changed = store.delete_torrent_download(&game_id)? || changed;
    logging::log_event(
        &state.data_dir,
        "game_removed",
        &[("game_id", game_id.as_str())],
    );
    Ok(changed)
}

#[tauri::command]
pub async fn redownload_asset(
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<DownloadRecord, String> {
    let current = {
        let store = lock_store(&state)?;
        store.get_download(&asset_id)?
    };
    if let Some(path) = current.and_then(|record| record.local_path) {
        let candidate = PathBuf::from(path);
        let _ = remove_path_if_allowed(&state.data_dir, &download_root(&state)?, &candidate);
    }
    download_asset(asset_id, state).await
}

#[tauri::command]
pub fn open_game_folder(game_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let path = {
        let store = lock_store(&state)?;
        store
            .get_download(&game_id)?
            .and_then(|download| download.local_path)
            .or_else(|| {
                store
                    .get_torrent_download(&game_id)
                    .ok()
                    .flatten()
                    .map(|record| record.save_dir)
            })
            .ok_or_else(|| format!("Game is not downloaded: {game_id}"))?
    };
    open_path(Path::new(&path))
}

#[tauri::command]
pub fn open_emulator_folder(platform: String, state: State<'_, AppState>) -> Result<(), String> {
    validate_platform(&platform)?;
    let exe_path = lock_store(&state)?
        .get_emulator_config(&platform)?
        .and_then(|config| config.exe_path)
        .ok_or_else(|| format!("No emulator configured for {platform}"))?;
    let parent = Path::new(&exe_path)
        .parent()
        .ok_or_else(|| format!("Emulator path has no parent directory: {exe_path}"))?;
    open_path(parent)
}

#[tauri::command]
pub fn open_logs_folder(state: State<'_, AppState>) -> Result<(), String> {
    open_path(&logging::log_dir(&state.data_dir))
}

#[tauri::command]
pub async fn run_health_check(state: State<'_, AppState>) -> Result<HealthReport, String> {
    build_health_report(&state)
}

#[tauri::command]
pub fn get_diagnostics_bundle(state: State<'_, AppState>) -> Result<DiagnosticsBundle, String> {
    let health = build_health_report(&state)?;
    let downloads = state.torrents.list_downloads()?;
    let log_path = logging::log_file_path(&state.data_dir);
    Ok(DiagnosticsBundle {
        generated_at: Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        data_dir: state.data_dir.to_string_lossy().to_string(),
        log_path: log_path.to_string_lossy().to_string(),
        health,
        downloads,
        logs: logging::tail_log(&state.data_dir, 500),
    })
}

pub(crate) fn build_requirements_report(
    store: &RepositoryStore,
    data_dir: &Path,
    game: &CatalogGameView,
) -> Result<RequirementsReport, String> {
    let game_downloaded = store
        .get_download(&game.id)?
        .and_then(|download| download.local_path)
        .map(|local_path| Path::new(&local_path).exists())
        .unwrap_or(false);
    let assets = store.get_assets(&game.required_system_file_ids)?;
    let mut requirements = Vec::new();

    for asset in assets {
        let download = store.get_download(&asset.id)?;
        let trusted = store.get_trusted_executable(&asset.id)?;
        let installation = inspect_asset_installation(store, data_dir, &asset, download.as_ref())?;
        let downloaded = installation.status == "ready";
        let trusted_ok = if asset.executable {
            trusted.is_some()
        } else {
            true
        };
        requirements.push(RequirementItem {
            asset,
            status: installation.status,
            downloaded,
            trusted: trusted_ok,
            local_path: download.and_then(|record| record.local_path),
            target_path: installation.target_path,
            sha256: installation.sha256,
            message: installation.message,
        });
    }

    let ready = game_downloaded
        && requirements
            .iter()
            .all(|item| item.downloaded && item.trusted);
    Ok(RequirementsReport {
        game_id: game.id.clone(),
        ready,
        game_downloaded,
        requirements,
    })
}

fn build_library_status(
    store: &RepositoryStore,
    data_dir: &Path,
    game: &CatalogGameView,
) -> Result<LibraryGameStatus, String> {
    let requirements = build_requirements_report(store, data_dir, game)?;
    let download = store.get_torrent_download(&game.id)?;
    let installed = requirements.game_downloaded
        || download
            .as_ref()
            .map(|record| record.status == "completed")
            .unwrap_or(false);
    let mut missing_requirements = Vec::new();

    for item in requirements.requirements {
        match item.status.as_str() {
            "ready" => {
                if !item.trusted {
                    missing_requirements
                        .push(format!("{} is not trusted", item.asset.display_name));
                }
            }
            "corrupt" => {
                missing_requirements.push(format!("{} is corrupt", item.asset.display_name));
            }
            "blocked" => {
                missing_requirements.push(item.message.unwrap_or_else(|| {
                    format!("{} cannot be installed yet", item.asset.display_name)
                }));
            }
            "error" => {
                missing_requirements.push(
                    item.message.unwrap_or_else(|| {
                        format!("{} installation failed", item.asset.display_name)
                    }),
                );
            }
            _ => {
                missing_requirements.push(format!("{} is not installed", item.asset.display_name));
            }
        }
    }

    Ok(LibraryGameStatus {
        game_id: game.id.clone(),
        installed,
        system_requirements_ready: missing_requirements.is_empty(),
        missing_requirements,
        download,
    })
}

struct BlockedAssetTarget {
    target_path: Option<String>,
    message: String,
}

fn resolve_asset_target(
    store: &RepositoryStore,
    data_dir: &Path,
    asset: &AssetView,
    source: &SourceUri,
) -> Result<PathBuf, BlockedAssetTarget> {
    let Some(hint) = asset.install_hint.as_ref() else {
        return Ok(destination_for_source(
            &data_dir.join("System"),
            &asset.platform,
            &asset.id,
            source,
            &asset.display_name,
        ));
    };

    match &hint.target {
        InstallTarget::AppSystem => {
            let Some(relative_path) = safe_relative_path_or_default(
                hint.relative_path.as_deref(),
                &file_name_for_source(source, &asset.display_name),
            ) else {
                return Err(blocked_asset(None, "System file install path is invalid."));
            };
            Ok(data_dir
                .join("System")
                .join(&asset.platform)
                .join(relative_path))
        }
        InstallTarget::EmulatorDir => {
            let config = store
                .get_emulator_config(&asset.platform)
                .map_err(|message| blocked_asset(None, message))?
                .ok_or_else(|| {
                    blocked_asset(
                        None,
                        format!(
                            "Configure the emulator for {} before installing {}.",
                            asset.platform, asset.display_name
                        ),
                    )
                })?;
            let exe_path = config.exe_path.as_deref().ok_or_else(|| {
                blocked_asset(
                    None,
                    format!(
                        "Configure the emulator for {} before installing {}.",
                        asset.platform, asset.display_name
                    ),
                )
            })?;
            let emulator_path = Path::new(exe_path);
            if validate_emulator_status(Some(exe_path)) != "valid" {
                return Err(blocked_asset(
                    Some(exe_path.to_string()),
                    format!(
                        "Emulator path for {} is not valid: {}",
                        asset.platform, exe_path
                    ),
                ));
            }
            let parent = emulator_path.parent().ok_or_else(|| {
                blocked_asset(
                    Some(exe_path.to_string()),
                    format!("Emulator path has no parent directory: {exe_path}"),
                )
            })?;
            let Some(relative_path) = safe_relative_path_or_default(
                hint.relative_path.as_deref(),
                &file_name_for_source(source, &asset.display_name),
            ) else {
                return Err(blocked_asset(None, "System file install path is invalid."));
            };
            Ok(parent.join(relative_path))
        }
        InstallTarget::UserSelected => Err(blocked_asset(
            None,
            format!(
                "{} requires a user-selected install path, which is not supported in v1.",
                asset.display_name
            ),
        )),
    }
}

fn inspect_asset_installation(
    store: &RepositoryStore,
    data_dir: &Path,
    asset: &AssetView,
    download: Option<&DownloadRecord>,
) -> Result<crate::schema::AssetInstallation, String> {
    let source = asset.sources.first();
    let expected_sha256 = expected_asset_sha256(asset);
    let target = match download
        .filter(|record| record.status == "ready")
        .and_then(|record| record.local_path.as_ref())
        .map(PathBuf::from)
    {
        Some(path) => Ok(path),
        None => {
            if let Some(source) = source {
                resolve_asset_target(store, data_dir, asset, source)
            } else {
                Err(blocked_asset(
                    None,
                    format!("{} has no sources.", asset.display_name),
                ))
            }
        }
    };

    let target = match target {
        Ok(path) => path,
        Err(blocked) => {
            return store.record_asset_installation(
                &asset.id,
                blocked.target_path.as_deref(),
                "blocked",
                None,
                Some(&blocked.message),
            );
        }
    };
    let target_path = target.to_string_lossy().to_string();

    if !target.exists() {
        return store.record_asset_installation(
            &asset.id,
            Some(&target_path),
            "missing",
            None,
            None,
        );
    }

    if !target.is_file() {
        return store.record_asset_installation(
            &asset.id,
            Some(&target_path),
            "error",
            None,
            Some("System file target is not a file."),
        );
    }

    if let Some(expected_sha256) = expected_sha256 {
        let actual_sha256 = hash_file(&target)?;
        if actual_sha256.eq_ignore_ascii_case(expected_sha256) {
            store.record_asset_installation(
                &asset.id,
                Some(&target_path),
                "ready",
                Some(&actual_sha256),
                None,
            )
        } else {
            store.record_asset_installation(
                &asset.id,
                Some(&target_path),
                "corrupt",
                Some(&actual_sha256),
                Some(&format!(
                    "SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
                )),
            )
        }
    } else {
        store.record_asset_installation(&asset.id, Some(&target_path), "ready", None, None)
    }
}

fn expected_asset_sha256(asset: &AssetView) -> Option<&str> {
    asset.sources.iter().find_map(|source| match source {
        SourceUri::Http { sha256, .. } => Some(sha256.as_str()),
        SourceUri::Magnet { .. } => None,
        SourceUri::UserProvided { sha256, .. } => sha256.as_deref(),
    })
}

fn blocked_asset(target_path: Option<String>, message: impl Into<String>) -> BlockedAssetTarget {
    BlockedAssetTarget {
        target_path,
        message: message.into(),
    }
}

fn safe_relative_path_or_default(input: Option<&str>, fallback_file_name: &str) -> Option<PathBuf> {
    let raw = input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_file_name);
    let path = Path::new(raw);
    if path.is_absolute() {
        return None;
    }

    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => sanitized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if sanitized.as_os_str().is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn validate_emulator_status(exe_path: Option<&str>) -> &'static str {
    let Some(exe_path) = exe_path.map(str::trim).filter(|value| !value.is_empty()) else {
        return "invalid";
    };
    let path = Path::new(exe_path);
    if !path.exists() {
        return "missing";
    }
    if !path.is_file() {
        return "invalid";
    }
    if cfg!(windows)
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| !extension.eq_ignore_ascii_case("exe"))
            .unwrap_or(true)
    {
        return "invalid";
    }

    "valid"
}

fn normalize_launch_args_template(value: Option<String>) -> Option<String> {
    value
        .map(|template| template.trim().to_string())
        .filter(|template| !template.is_empty())
}

async fn fetch_repository_schema(
    url: &str,
    allow_dev_http: bool,
) -> Result<RepositorySchema, String> {
    let parsed = validate_repository_url(url, allow_dev_http)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| format!("Failed to initialize repository client: {error}"))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| format!("Failed to fetch repository: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Repository returned an error: {error}"))?;
    let repo = response
        .json::<RepositorySchema>()
        .await
        .map_err(|error| format!("Repository JSON is invalid: {error}"))?;
    validate_repository_schema(&repo, allow_dev_http)?;
    Ok(repo)
}

fn build_repository_preview(url: &str, repo: &RepositorySchema) -> RepositoryPreview {
    let raw_json = serde_json::to_vec(repo).unwrap_or_default();
    let content_hash = repo
        .metadata
        .content_hash
        .clone()
        .unwrap_or_else(|| hex::encode(Sha256::digest(&raw_json)));
    RepositoryPreview {
        url: url.to_string(),
        id: repo.metadata.id.clone(),
        name: repo.metadata.name.clone(),
        version: repo.metadata.version.clone(),
        maintainer: repo.metadata.maintainer.clone(),
        homepage_url: repo.metadata.homepage_url.clone(),
        license: repo.metadata.license.clone(),
        trust_level: repo
            .metadata
            .trust_level
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        catalog_count: repo.catalog.len(),
        system_file_count: repo.system_files.len(),
        has_executable_assets: repo.system_files.iter().any(|asset| asset.executable),
        content_hash,
    }
}

fn download_root(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    let configured = lock_store(state)?.get_config("download_root")?;
    Ok(configured
        .map(PathBuf::from)
        .unwrap_or_else(|| state.data_dir.join("Games")))
}

fn preflight_disk_space(root: &Path, needed_bytes: Option<u64>) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("Failed to create download folder: {error}"))?;
    let Some(needed_bytes) = needed_bytes else {
        return Ok(());
    };
    let free = fs2::available_space(root)
        .map_err(|error| format!("Failed to inspect free disk space: {error}"))?;
    let buffer = 1024_u64 * 1024 * 1024;
    if free < needed_bytes.saturating_add(buffer) {
        return Err(format!(
            "Insufficient disk space: need {:.2} GB plus 1 GB buffer, but only {:.2} GB is free.",
            needed_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
            free as f64 / 1024.0 / 1024.0 / 1024.0
        ));
    }
    Ok(())
}

fn emit_http_download_completed(
    app: &AppHandle,
    game_id: &str,
    save_dir: &str,
    total_bytes: u64,
) -> Result<(), String> {
    let _ = app.emit(
        "download:progress",
        crate::torrent::DownloadProgressEvent {
            game_id: game_id.to_string(),
            status: "completed".to_string(),
            progress: 1.0,
            progress_percent: 100.0,
            downloaded_bytes: total_bytes,
            total_bytes,
            download_speed_bytes_per_sec: 0,
            upload_speed_bytes_per_sec: 0,
            peers_count: 0,
            finished: true,
            save_dir: save_dir.to_string(),
            error: None,
        },
    );
    Ok(())
}

fn build_health_report(state: &State<'_, AppState>) -> Result<HealthReport, String> {
    let store = lock_store(state)?;
    let repositories = store.list_repositories()?;
    let catalog = store.get_catalog()?;
    let configs = store.list_emulator_configs()?;
    let downloads = store.list_download_records()?;
    let torrent_downloads = store.list_torrent_downloads()?;
    let mut system_file_ids = HashSet::new();
    let mut system_files = Vec::new();
    let mut game_files = Vec::new();

    let emulators = MVP_PLATFORM_CONFIGS
        .iter()
        .map(|platform| {
            let config = configs
                .iter()
                .find(|config| config.platform == platform.id);
            let path = config.and_then(|config| config.exe_path.clone());
            let status = match path.as_deref().map(|path| validate_emulator_status(Some(path))) {
                Some("valid") => "ready",
                Some("missing") => "missing",
                _ => "missing",
            };
            HealthCheckItem {
                id: format!("emulator:{}", platform.id),
                label: format!("{} ({})", platform.emulator_name, platform.label),
                status: status.to_string(),
                message: if status == "ready" {
                    Some(platform.executable_hint.to_string())
                } else {
                    Some(format!("Expected executable: {}", platform.executable_hint))
                },
                action: Some(if status == "ready" {
                    "openEmulatorFolder".to_string()
                } else {
                    "reconfigureEmulator".to_string()
                }),
                path,
            }
        })
        .collect::<Vec<_>>();

    for game in &catalog {
        if let Some(download) = downloads.iter().find(|download| download.subject_id == game.id) {
            let path = download.local_path.clone().unwrap_or_default();
            let game_status = check_game_file_health(&path, &game.expected_extensions);
            game_files.push(HealthCheckItem {
                id: format!("game:{}", game.id),
                label: game.title.clone(),
                status: game_status.0,
                message: game_status.1,
                action: Some("openGameFolder".to_string()),
                path: Some(path),
            });
        } else if let Some(torrent) = torrent_downloads
            .iter()
            .find(|download| download.game_id == game.id && download.status == "completed")
        {
            let game_status = check_game_file_health(&torrent.save_dir, &game.expected_extensions);
            game_files.push(HealthCheckItem {
                id: format!("game:{}", game.id),
                label: game.title.clone(),
                status: game_status.0,
                message: game_status.1,
                action: Some("openGameFolder".to_string()),
                path: Some(torrent.save_dir.clone()),
            });
        }

        let requirements = build_requirements_report(&store, &state.data_dir, game)?;
        for item in requirements.requirements {
            if !system_file_ids.insert(item.asset.id.clone()) {
                continue;
            }
            system_files.push(HealthCheckItem {
                id: format!("asset:{}", item.asset.id),
                label: item.asset.display_name,
                status: match item.status.as_str() {
                    "ready" if item.trusted => "ready",
                    "corrupt" => "corrupt",
                    "blocked" => "blocked",
                    "error" => "error",
                    _ => "missing",
                }
                .to_string(),
                message: item.message.or_else(|| item.target_path.clone()),
                action: Some(match item.status.as_str() {
                    "corrupt" | "error" => "redownloadAsset",
                    "ready" if !item.trusted => "trustExecutable",
                    _ => "openTargetFolder",
                }
                .to_string()),
                path: item.target_path,
            });
        }
    }

    let repositories = repositories
        .into_iter()
        .map(|repository| HealthCheckItem {
            id: format!("repository:{}", repository.id),
            label: repository.name,
            status: "ready".to_string(),
            message: Some(format!(
                "{} games / {} system files / {}",
                repository.catalog_count, repository.system_file_count, repository.url
            )),
            action: Some("refreshRepository".to_string()),
            path: Some(repository.url),
        })
        .collect::<Vec<_>>();

    let active_downloads = torrent_downloads
        .iter()
        .filter(|download| matches!(download.status.as_str(), "resolving" | "downloading"))
        .count();
    let downloader = HealthCheckItem {
        id: "downloader:librqbit".to_string(),
        label: "Downloader session".to_string(),
        status: "ready".to_string(),
        message: Some(format!("{active_downloads} active torrent download(s)")),
        action: None,
        path: Some(download_root(state)?.to_string_lossy().to_string()),
    };

    Ok(HealthReport {
        generated_at: Utc::now().to_rfc3339(),
        emulators,
        system_files,
        game_files,
        repositories,
        downloader,
    })
}

fn check_game_file_health(path: &str, expected_extensions: &[String]) -> (String, Option<String>) {
    if path.trim().is_empty() {
        return ("missing".to_string(), Some("No local path recorded.".to_string()));
    }
    let path = Path::new(path);
    if !path.exists() {
        return (
            "missing".to_string(),
            Some(format!("Game file or folder not found: {}", path.display())),
        );
    }
    if path.is_file() {
        let ok = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!(".{}", extension.to_lowercase()))
            .map(|extension| {
                expected_extensions
                    .iter()
                    .any(|expected| expected.eq_ignore_ascii_case(&extension))
            })
            .unwrap_or(false);
        return if ok {
            ("ready".to_string(), Some(path.display().to_string()))
        } else {
            (
                "error".to_string(),
                Some(format!(
                    "Downloaded file does not match expected extensions: {}",
                    expected_extensions.join(", ")
                )),
            )
        };
    }
    if find_matching_game_file(path, expected_extensions).is_some() {
        ("ready".to_string(), Some(path.display().to_string()))
    } else {
        (
            "error".to_string(),
            Some(format!(
                "Download complete, but no game file was found matching {}.",
                expected_extensions.join(", ")
            )),
        )
    }
}

fn find_matching_game_file(root: &Path, expected_extensions: &[String]) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_matching_game_file(&path, expected_extensions) {
                return Some(found);
            }
            continue;
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!(".{}", extension.to_lowercase()))?;
        if expected_extensions
            .iter()
            .any(|expected| expected.eq_ignore_ascii_case(&extension))
        {
            return Some(path);
        }
    }
    None
}

fn remove_path_if_allowed(data_dir: &Path, download_root: &Path, candidate: &Path) -> Result<(), String> {
    if !candidate.exists() {
        return Ok(());
    }
    let canonical_candidate = fs::canonicalize(candidate)
        .map_err(|error| format!("Failed to inspect {}: {error}", candidate.display()))?;
    let canonical_data_dir = fs::canonicalize(data_dir)
        .map_err(|error| format!("Failed to inspect app data directory: {error}"))?;
    let canonical_download_root = fs::canonicalize(download_root)
        .map_err(|error| format!("Failed to inspect download folder: {error}"))?;
    if !canonical_candidate.starts_with(&canonical_data_dir)
        && !canonical_candidate.starts_with(&canonical_download_root)
    {
        return Err(format!(
            "Refusing to delete files outside RetroHydra folders: {}",
            canonical_candidate.display()
        ));
    }
    if canonical_candidate.is_dir() {
        fs::remove_dir_all(&canonical_candidate)
    } else {
        fs::remove_file(&canonical_candidate)
    }
    .map_err(|error| format!("Failed to remove {}: {error}", canonical_candidate.display()))
}

fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    }
    Ok(())
}

fn lock_store<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, RepositoryStore>, String> {
    state
        .store
        .lock()
        .map_err(|_| "Repository store lock is poisoned.".to_string())
}

#[allow(dead_code)]
fn source_has_http(sources: &[SourceUri]) -> bool {
    sources
        .iter()
        .any(|source| matches!(source, SourceUri::Http { .. }))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::build_library_status;
    use crate::schema::{
        AssetKind, RepositoryAsset, RepositoryGame, RepositoryMetadata, RepositorySchema, SourceUri,
    };
    use crate::storage::RepositoryStore;
    use sha2::{Digest, Sha256};

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
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
                downloads: vec![SourceUri::Magnet {
                    uri: "magnet:?xt=urn:btih:abc".to_string(),
                    info_hash: None,
                    size_bytes: None,
                }],
                expected_extensions: vec![".nes".to_string()],
                required_system_file_ids: required_asset_ids,
            }],
        }
    }

    fn open_store(required_asset_ids: Vec<String>) -> (tempfile::TempDir, RepositoryStore) {
        let dir = tempdir().unwrap();
        let mut store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();
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
        std::fs::write(&game_path, b"game").unwrap();
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
        std::fs::write(&game_path, b"game").unwrap();
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
    fn completed_torrent_download_counts_as_installed_through_legacy_record() {
        let (dir, store) = open_store(Vec::new());
        let game = store.get_game("repo::game").unwrap().unwrap();
        let game_dir = dir.path().join("downloaded-game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let game_dir_string = game_dir.to_string_lossy().to_string();
        store
            .upsert_torrent_download_start(
                "repo::game",
                "magnet:?xt=urn:btih:abc",
                &game_dir_string,
            )
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
}
