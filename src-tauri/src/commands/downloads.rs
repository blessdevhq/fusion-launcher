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
pub fn import_asset_file(
    asset_id: String,
    source_path: String,
    state: State<'_, AppState>,
) -> Result<ImportAssetFileReport, String> {
    let store = match lock_store(&state) {
        Ok(store) => store,
        Err(_) => {
            return Ok(import_asset_error("", "store_failed"));
        }
    };

    Ok(import_asset_file_into_store(
        &store,
        &state.data_dir,
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
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<GameDownloadStartReport, String> {
    start_game_download_internal(&game_id, &state, &app).await
}

pub(crate) async fn start_game_download_internal(
    game_id: &str,
    state: &AppState,
    app: &AppHandle,
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
                torrent: state.torrents.get_game_download(game_id)?,
            })
        }
        SourceUri::Http { size_bytes, .. } | SourceUri::Bundled { size_bytes, .. } => {
            let source_kind = direct_source_kind(source);
            let destination = destination_for_source(
                &download_root,
                &game.platform,
                &game.id,
                source,
                &game.title,
            );
            let target_path = destination.to_string_lossy().to_string();
            let expected_bytes = size_bytes.unwrap_or(0);

            if let Err(error) = preflight_disk_space(&download_root, *size_bytes) {
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
    changed = store.delete_scrape_artifacts(&game_id)? || changed;
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
    open_path(&folder_path_for_open(Path::new(&path))?)
}
