pub(super) fn download_root(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    download_root_for_app_state(state)
}

pub(super) fn managed_storage_root_for_app_state(state: &AppState) -> PathBuf {
    state.data_dir.clone()
}

pub(super) fn default_games_root(data_dir: &Path) -> PathBuf {
    data_dir.join("Games")
}

pub(super) fn emulators_root(data_dir: &Path) -> PathBuf {
    data_dir.join("Emulators")
}

pub(super) fn system_root(data_dir: &Path) -> PathBuf {
    data_dir.join("System")
}

pub(crate) fn temp_root(data_dir: &Path) -> PathBuf {
    data_dir.join("Temp")
}

pub(super) fn download_root_for_app_state(state: &AppState) -> Result<PathBuf, String> {
    let configured = lock_app_store(state)?.get_config("download_root")?;
    Ok(configured
        .map(PathBuf::from)
        .unwrap_or_else(|| default_games_root(&managed_storage_root_for_app_state(state))))
}

pub(crate) fn resolve_target_dir_override(target_dir: Option<&str>) -> Result<Option<PathBuf>, String> {
    let Some(target_dir) = target_dir.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(target_dir);
    if !path.is_absolute() {
        return Err("Download destination must be an absolute path.".to_string());
    }
    fs::create_dir_all(&path)
        .map_err(|error| format!("Failed to create download destination: {error}"))?;
    fs::canonicalize(&path)
        .or(Ok(path))
        .map(Some)
}

pub(super) fn preflight_disk_space(root: &Path, needed_bytes: Option<u64>) -> Result<(), String> {
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

pub(super) fn emit_direct_download_record(app: &AppHandle, record: &TorrentDownloadRecord) {
    let _ = app.emit(
        "download:progress",
        crate::torrent::DownloadProgressEvent {
            game_id: record.game_id.clone(),
            subject_type: record.subject_type.clone(),
            display_name: record.display_name.clone(),
            status: record.status.clone(),
            progress: record.progress_percent / 100.0,
            progress_percent: record.progress_percent,
            downloaded_bytes: record.downloaded_bytes,
            total_bytes: record.total_bytes,
            download_speed_bytes_per_sec: record.download_speed_bytes_per_sec,
            upload_speed_bytes_per_sec: record.upload_speed_bytes_per_sec,
            peers_count: record.peers_count,
            finished: record.status == "completed",
            save_dir: record.save_dir.clone(),
            error: record.error_message.clone(),
        },
    );
}

pub(super) fn spawn_metadata_scrape(app: &AppHandle, state: &AppState, game_id: String) {
    let app = app.clone();
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        let _ = crate::scraper::scrape_game(app, state, game_id).await;
    });
}
