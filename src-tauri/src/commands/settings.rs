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
