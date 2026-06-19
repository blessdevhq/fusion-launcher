#[tauri::command]
pub async fn download_asset(
    asset_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DownloadRecord, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
    download_asset_internal(&asset_id, state.inner(), &app, target_dir.as_deref()).await
}

pub(crate) async fn download_asset_internal(
    asset_id: &str,
    state: &AppState,
    app: &AppHandle,
    target_dir: Option<&Path>,
) -> Result<DownloadRecord, String> {
    let content_dir = library_root_for_app_state(state);
    let asset = {
        let store = lock_app_store(state)?;
        store
            .get_asset(asset_id)?
            .ok_or_else(|| format!("Unknown asset: {asset_id}"))?
    };

    let source = asset
        .sources
        .first()
        .ok_or_else(|| format!("Asset {} has no sources.", asset.display_name))?;
    if matches!(source, SourceUri::UserProvided { .. }) {
        let destination = {
            let store = lock_app_store(state)?;
            resolve_asset_target_with_override(&store, &content_dir, &asset, source, target_dir)
                .map_err(|blocked| blocked.message)?
        };
        let target_path = destination.to_string_lossy().to_string();
        let message = format!(
            "{} is user-provided. Place the file at {} or use Import file.",
            asset.display_name, target_path
        );
        let store = lock_app_store(state)?;
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
        let store = lock_app_store(state)?;
        match resolve_asset_target_with_override(&store, &content_dir, &asset, source, target_dir) {
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

    let source_kind = direct_source_kind(source);
    let target_path = destination.to_string_lossy().to_string();
    let expected_bytes = source_size_bytes(source).unwrap_or(0);

    let started = lock_app_store(state)?.record_direct_asset_download_started(
        &asset.id,
        source_kind,
        &target_path,
        expected_bytes,
    )?;
    emit_direct_download_record(app, &started);
    let download_result = {
        let asset_id = asset.id.clone();
        crate::downloads::download_source_to_file_with_progress(
            source,
            &destination,
            |downloaded, total| {
                let percent = match total {
                    Some(total) if total > 0 => (downloaded as f64 / total as f64) * 100.0,
                    _ => 0.0,
                };
                if let Ok(store) = lock_app_store(state) {
                    if let Ok(record) = store.update_torrent_progress(
                        &asset_id,
                        "downloading",
                        percent,
                        downloaded,
                        total.unwrap_or(0),
                        0,
                        0,
                        0,
                    ) {
                        drop(store);
                        emit_direct_download_record(app, &record);
                    }
                }
            },
        )
        .await
    };

    match download_result {
        Ok(file) => {
            let total_bytes = source_size_bytes(source).unwrap_or_else(|| file_size(&file.path));
            let final_path = if is_manifest_emulator_bundle(&asset) {
                match post_process_manifest_emulator_bundle_archive(&asset, &file.path).await {
                    Ok(path) => path,
                    Err(error) => {
                        let target_path = file.path.to_string_lossy().to_string();
                        let store = lock_app_store(state)?;
                        let _ = store.record_asset_installation(
                            &asset.id,
                            Some(&target_path),
                            "error",
                            Some(&file.sha256),
                            Some(&error),
                        );
                        if let Ok(record) = store.record_direct_asset_download_failed(
                            &asset.id,
                            source_kind,
                            &target_path,
                            expected_bytes,
                            &error,
                        ) {
                            drop(store);
                            emit_direct_download_record(app, &record);
                        }
                        return Err(error);
                    }
                }
            } else {
                file.path.clone()
            };
            record_completed_asset_download(
                state,
                app,
                &asset,
                source_kind,
                &final_path,
                Some(&file.sha256),
                total_bytes,
            )
        }
        Err(error) => {
            let store = lock_app_store(state)?;
            let target_path = destination.to_string_lossy().to_string();
            let _ = store.record_asset_installation(
                &asset.id,
                Some(&target_path),
                "error",
                None,
                Some(&error),
            );
            if let Ok(record) = store.record_direct_asset_download_failed(
                &asset.id,
                source_kind,
                &target_path,
                expected_bytes,
                &error,
            ) {
                drop(store);
                emit_direct_download_record(app, &record);
            }
            Err(error)
        }
    }
}

pub(crate) async fn prepare_manifest_emulator_bundle_asset(
    asset_id: &str,
    state: &AppState,
    app: &AppHandle,
    target_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    let content_dir = library_root_for_app_state(state);
    let (asset, existing, expected_bundle_dir) = {
        let store = lock_app_store(state)?;
        let asset = store
            .get_asset(asset_id)?
            .ok_or_else(|| format!("Unknown asset: {asset_id}"))?;
        if !is_manifest_emulator_bundle(&asset) {
            return Err(format!(
                "asset_is_not_manifest_emulator_bundle:{}",
                asset.id
            ));
        }
        let source = asset
            .sources
            .first()
            .ok_or_else(|| format!("Asset {} has no sources.", asset.display_name))?;
        let expected_bundle_dir = resolve_asset_target_with_override(
            &store,
            &content_dir,
            &asset,
            source,
            target_dir,
        )
        .map_err(|blocked| blocked.message)?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("Emulator asset target has no parent: {}", asset.id))?;
        let existing = store
            .get_download(&asset.id)?
            .filter(|record| is_download_ready_status(&record.status));
        (asset, existing, expected_bundle_dir)
    };

    if let Some(record) = existing {
        let path = record
            .local_path
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| format!("emulator_asset_download_missing_path:{asset_id}"))?;
        if path.is_dir() && path_is_existing_bundle_dir(&expected_bundle_dir, &path) {
            return Ok(path);
        }
        if path.is_file()
            && path
                .parent()
                .is_some_and(|parent| path_is_existing_bundle_dir(&expected_bundle_dir, parent))
        {
            let total_bytes = file_size(&path);
            let source_kind = asset
                .sources
                .first()
                .map(direct_source_kind)
                .unwrap_or("http");
            let final_path = post_process_manifest_emulator_bundle_archive(&asset, &path).await?;
            let record = record_completed_asset_download(
                state,
                app,
                &asset,
                source_kind,
                &final_path,
                record.sha256.as_deref(),
                total_bytes,
            )?;
            return record
                .local_path
                .map(PathBuf::from)
                .ok_or_else(|| format!("emulator_asset_download_missing_path:{asset_id}"));
        }
    }

    let record = download_asset_internal(asset_id, state, app, target_dir).await?;
    let path = record
        .local_path
        .map(PathBuf::from)
        .ok_or_else(|| format!("emulator_asset_download_missing_path:{asset_id}"))?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!(
            "emulator_asset_not_extracted:{}",
            path.display()
        ))
    }
}

fn record_completed_asset_download(
    state: &AppState,
    app: &AppHandle,
    asset: &AssetView,
    source_kind: &str,
    local_path: &Path,
    sha256: Option<&str>,
    total_bytes: u64,
) -> Result<DownloadRecord, String> {
    let local_path = local_path.to_string_lossy().to_string();
    let store = lock_app_store(state)?;
    store.record_asset_installation(&asset.id, Some(&local_path), "ready", sha256, None)?;
    if let Some(sha256) = sha256 {
        let (record, torrent) = store.record_direct_asset_download_completed(
            &asset.id,
            source_kind,
            &local_path,
            sha256,
            total_bytes,
        )?;
        drop(store);
        emit_direct_download_record(app, &torrent);
        Ok(record)
    } else {
        store.record_download(&asset.id, "asset", Some(&local_path), None, None)
    }
}

async fn post_process_manifest_emulator_bundle_archive(
    asset: &AssetView,
    archive_path: &Path,
) -> Result<PathBuf, String> {
    let asset = asset.clone();
    let archive_path = archive_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        extract_manifest_emulator_bundle_archive(&asset, &archive_path)
    })
    .await
    .map_err(|error| format!("Emulator bundle extraction task failed: {error}"))?
}

pub(super) fn extract_manifest_emulator_bundle_archive(
    asset: &AssetView,
    archive_path: &Path,
) -> Result<PathBuf, String> {
    if !is_manifest_emulator_bundle(asset) {
        return Err(format!(
            "asset_is_not_manifest_emulator_bundle:{}",
            asset.id
        ));
    }
    let extension = archive_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("zip") {
        return Err(format!(
            "unsupported_manifest_emulator_archive:{}",
            archive_path.display()
        ));
    }
    let extract_dir = archive_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("Emulator archive has no parent: {}", archive_path.display()))?;
    crate::archive::extract_archive_safely(archive_path, &extract_dir)?;
    fs::remove_file(archive_path)
        .map_err(|error| format!("Failed to remove emulator zip archive: {error}"))?;
    Ok(extract_dir)
}

fn path_is_existing_bundle_dir(expected: &Path, actual: &Path) -> bool {
    if !expected.exists() || !actual.exists() {
        return false;
    }
    match (fs::canonicalize(expected), fs::canonicalize(actual)) {
        (Ok(expected), Ok(actual)) => expected == actual,
        _ => false,
    }
}

#[tauri::command]
pub fn import_asset_file(
    asset_id: String,
    source_path: String,
    state: State<'_, AppState>,
) -> Result<ImportAssetFileReport, String> {
    let content_dir = library_root_for_app_state(&state);
    let store = match lock_store(&state) {
        Ok(store) => store,
        Err(_) => {
            return Ok(import_asset_error("", "store_failed"));
        }
    };

    Ok(import_asset_file_into_store(
        &store,
        &content_dir,
        &asset_id,
        Path::new(source_path.trim()),
    ))
}

#[tauri::command]
pub fn import_game_file(
    game_id: String,
    source_path: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ImportGameFileReport, String> {
    let download_root = match download_root(&state) {
        Ok(path) => path,
        Err(_) => return Ok(import_game_error(&game_id, "", "store_failed", None)),
    };
    let store = match lock_store(&state) {
        Ok(store) => store,
        Err(_) => return Ok(import_game_error("", "", "store_failed", None)),
    };

    let report = import_game_file_into_store(
        &store,
        &download_root,
        &game_id,
        Path::new(source_path.trim()),
    );
    if report.error_code.is_none() {
        spawn_metadata_scrape(&app, state.inner(), report.game_id.clone());
    }
    Ok(report)
}

#[tauri::command]
pub async fn download_game(
    game_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<DownloadRecord, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
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
    if matches!(
        game.content_mode.as_deref(),
        Some("user_provided" | "metadata_only")
    ) || matches!(source, SourceUri::UserProvided { .. })
    {
        return Err(format!(
            "{} is user-provided content. Import your local game file instead of starting an automatic download.",
            game.title
        ));
    }
    let destination = game_direct_download_destination(
        download_root_for_app_state(state.inner())?,
        target_dir.as_deref(),
        &game,
        source,
    );

    match download_source_to_file(source, &destination).await {
        Ok(file) => {
            let local_path = file.path.to_string_lossy().to_string();
            let total_bytes = source_size_bytes(source).unwrap_or_else(|| file_size(&file.path));
            let (record, _) = lock_store(&state)?.record_direct_game_download_completed(
                &game.id,
                direct_source_kind(source),
                &local_path,
                &file.sha256,
                total_bytes,
            )?;
            Ok(record)
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
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<GameDownloadStartReport, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
    start_game_download_internal(&game_id, &state, &app, target_dir.as_deref()).await
}

pub(crate) async fn start_game_download_internal(
    game_id: &str,
    state: &AppState,
    app: &AppHandle,
    target_dir: Option<&Path>,
) -> Result<GameDownloadStartReport, String> {
    let game = {
        let store = lock_app_store(state)?;
        store
            .get_game(game_id)?
            .ok_or_else(|| format!("Unknown game: {game_id}"))?
    };
    let source = game
        .downloads
        .first()
        .ok_or_else(|| format!("Game {} has no download sources.", game.title))?;
    if matches!(
        game.content_mode.as_deref(),
        Some("user_provided" | "metadata_only")
    ) || matches!(source, SourceUri::UserProvided { .. })
    {
        return Err(format!(
            "{} is user-provided content. Import your local game file instead of starting an automatic download.",
            game.title
        ));
    }
    let download_root = download_root_for_app_state(state)?;

    match source {
        SourceUri::Magnet {
            uri, size_bytes, ..
        } => {
            let save_root = target_dir.unwrap_or(download_root.as_path());
            preflight_disk_space(save_root, *size_bytes)?;
            let save_dir = game_torrent_save_dir(&download_root, target_dir, &game);
            let save_dir_string = save_dir.to_string_lossy().to_string();
            let torrent = state
                .torrents()?
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
                torrent: state.torrents()?.get_game_download(game_id)?,
            })
        }
        SourceUri::Http { size_bytes, .. } | SourceUri::Bundled { size_bytes, .. } => {
            let source_kind = direct_source_kind(source);
            let destination =
                game_direct_download_destination(download_root.clone(), target_dir, &game, source);
            let target_path = destination.to_string_lossy().to_string();
            let expected_bytes = size_bytes.unwrap_or(0);

            let preflight_root = target_dir.unwrap_or(download_root.as_path());
            if let Err(error) = preflight_disk_space(preflight_root, *size_bytes) {
                if let Ok(store) = lock_app_store(state) {
                    if let Ok(record) = store.record_direct_game_download_failed(
                        &game.id,
                        source_kind,
                        &target_path,
                        expected_bytes,
                        &error,
                    ) {
                        emit_direct_download_record(app, &record);
                    }
                }
                return Err(error);
            }

            lock_app_store(state)?.record_direct_game_download_started(
                &game.id,
                source_kind,
                &target_path,
                expected_bytes,
            )?;
            logging::log_event(
                &state.data_dir,
                "game_download_started",
                &[("game_id", game.id.as_str()), ("source", source_kind)],
            );

            let download_result = {
                let game_id = game.id.clone();
                crate::downloads::download_source_to_file_with_progress(
                    source,
                    &destination,
                    |downloaded, total| {
                        let percent = match total {
                            Some(total) if total > 0 => (downloaded as f64 / total as f64) * 100.0,
                            _ => 0.0,
                        };
                        if let Ok(store) = lock_app_store(state) {
                            if let Ok(record) = store.update_torrent_progress(
                                &game_id,
                                "downloading",
                                percent,
                                downloaded,
                                total.unwrap_or(0),
                                0,
                                0,
                                0,
                            ) {
                                drop(store);
                                emit_direct_download_record(app, &record);
                            }
                        }
                    },
                )
                .await
            };
            let file = match download_result {
                Ok(file) => file,
                Err(error) => {
                    let failed = lock_app_store(state)?.record_direct_game_download_failed(
                        &game.id,
                        source_kind,
                        &target_path,
                        expected_bytes,
                        &error,
                    )?;
                    emit_direct_download_record(app, &failed);
                    logging::log_event(
                        &state.data_dir,
                        "game_download_failed",
                        &[
                            ("game_id", game.id.as_str()),
                            ("source", source_kind),
                            ("error", error.as_str()),
                        ],
                    );
                    return Err(error);
                }
            };
            let local_path = file.path.to_string_lossy().to_string();
            let total_bytes = size_bytes.unwrap_or_else(|| file_size(&file.path));
            let (record, torrent) = lock_app_store(state)?.record_direct_game_download_completed(
                &game.id,
                source_kind,
                &local_path,
                &file.sha256,
                total_bytes,
            )?;
            emit_direct_download_record(app, &torrent);
            logging::log_event(
                &state.data_dir,
                "game_download_completed",
                &[("game_id", game.id.as_str()), ("source", source_kind)],
            );
            spawn_metadata_scrape(app, state, game.id.clone());
            Ok(GameDownloadStartReport {
                game_id: game.id,
                source_kind: source_kind.to_string(),
                save_dir: local_path,
                record: Some(record),
                torrent: Some(torrent),
            })
        }
        SourceUri::UserProvided { .. } => {
            Err("Game downloads cannot be user-provided.".to_string())
        }
    }
}

fn game_direct_download_destination(
    default_root: PathBuf,
    target_dir: Option<&Path>,
    game: &CatalogGameView,
    source: &SourceUri,
) -> PathBuf {
    if let Some(target_dir) = target_dir {
        return target_dir.join(file_name_for_source(source, &game.title));
    }
    destination_for_source(&default_root, &game.platform, &game.id, source, &game.title)
}

fn game_torrent_save_dir(
    default_root: &Path,
    target_dir: Option<&Path>,
    game: &CatalogGameView,
) -> PathBuf {
    match target_dir {
        Some(target_dir) => target_dir.join(crate::downloads::safe_segment(&game.id)),
        None => default_root
            .join(crate::downloads::safe_segment(&game.platform))
            .join(crate::downloads::safe_segment(&game.id)),
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
    if !is_download_ready_status(&download.status) {
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
pub async fn remove_game(
    game_id: String,
    delete_files: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    remove_download_internal(&game_id, delete_files, state.inner(), "game_removed").await
}

#[tauri::command]
pub async fn remove_download(
    download_id: String,
    delete_files: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    remove_download_internal(&download_id, delete_files, state.inner(), "download_removed").await
}

async fn remove_download_internal(
    download_id: &str,
    delete_files: bool,
    state: &AppState,
    event_name: &str,
) -> Result<bool, String> {
    let download = {
        let store = lock_app_store(state)?;
        (
            store.get_download(download_id)?,
            store.get_torrent_download(download_id)?,
        )
    };

    if let Some(torrent) = download.1.as_ref() {
        if !matches!(torrent.status.as_str(), "completed" | "cancelled") {
            if let Ok(torrents) = state.torrents() {
                let _ = torrents.cancel_download(download_id.to_string()).await;
            }
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
            remove_recorded_download_path(&candidate)?;
        }
    }

    let store = lock_app_store(state)?;
    let is_asset = download
        .0
        .as_ref()
        .is_some_and(|record| record.subject_type == "asset")
        || download
            .1
            .as_ref()
            .and_then(|record| record.subject_type.as_deref())
            == Some("asset");
    let mut changed = download.0.is_some() || download.1.is_some();
    if is_asset {
        store.delete_asset_state(download_id)?;
        changed = store.delete_torrent_download(download_id)? || changed;
    } else {
        changed = store.delete_download(download_id)? || changed;
        changed = store.delete_torrent_download(download_id)? || changed;
        changed = store.delete_scrape_artifacts(download_id)? || changed;
    }
    logging::log_event(
        &state.data_dir,
        event_name,
        &[("download_id", download_id)],
    );
    Ok(changed)
}

#[tauri::command]
pub async fn redownload_asset(
    asset_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DownloadRecord, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
    let current = {
        let store = lock_store(&state)?;
        store.get_download(&asset_id)?
    };
    if let Some(path) = current.and_then(|record| record.local_path) {
        let candidate = PathBuf::from(path);
        let _ = remove_recorded_download_path(&candidate);
    }
    download_asset_internal(&asset_id, state.inner(), &app, target_dir.as_deref()).await
}

#[tauri::command]
pub fn open_game_folder(game_id: String, state: State<'_, AppState>) -> Result<(), String> {
    open_download_folder_internal(&game_id, state.inner())
}

#[tauri::command]
pub fn open_download_folder(download_id: String, state: State<'_, AppState>) -> Result<(), String> {
    open_download_folder_internal(&download_id, state.inner())
}

fn open_download_folder_internal(download_id: &str, state: &AppState) -> Result<(), String> {
    let path = {
        let store = lock_app_store(state)?;
        store
            .get_download(download_id)?
            .and_then(|download| download.local_path)
            .or_else(|| {
                store
                    .get_torrent_download(download_id)
                    .ok()
                    .flatten()
                    .map(|record| record.save_dir)
            })
            .ok_or_else(|| format!("Download is not available: {download_id}"))?
    };
    open_path(&folder_path_for_open(Path::new(&path))?)
}
