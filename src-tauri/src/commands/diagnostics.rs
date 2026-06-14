#[tauri::command]
pub fn open_logs_folder(state: State<'_, AppState>) -> Result<(), String> {
    open_path(&logging::log_dir(&state.data_dir))
}

#[tauri::command]
pub async fn run_health_check(state: State<'_, AppState>) -> Result<HealthReport, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || build_health_report(&state))
        .await
        .map_err(|error| format!("Health check task failed: {error}"))?
}

#[tauri::command]
pub fn get_diagnostics_paths(state: State<'_, AppState>) -> DiagnosticsPaths {
    DiagnosticsPaths {
        data_dir: state.data_dir.to_string_lossy().to_string(),
        log_path: logging::log_file_path(&state.data_dir)
            .to_string_lossy()
            .to_string(),
    }
}

#[tauri::command]
pub async fn get_diagnostics_bundle(
    state: State<'_, AppState>,
) -> Result<DiagnosticsBundle, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || build_diagnostics_bundle(&state))
        .await
        .map_err(|error| format!("Diagnostics task failed: {error}"))?
}
