pub(super) fn download_root(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    download_root_for_app_state(state)
}

pub(super) fn download_root_for_app_state(state: &AppState) -> Result<PathBuf, String> {
    let configured = lock_app_store(state)?.get_config("download_root")?;
    Ok(configured
        .map(PathBuf::from)
        .unwrap_or_else(|| state.data_dir.join("Games")))
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
