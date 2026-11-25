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
pub async fn save_emulator_config(
    platform: String,
    exe_path: String,
    launch_args_template: Option<String>,
    state: State<'_, AppState>,
) -> Result<EmulatorConfig, String> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        save_emulator_config_internal(&platform, exe_path.trim(), launch_args_template, &state)
    })
    .await
    .map_err(|e| format!("Save emulator config task failed: {e}"))?
}

pub(crate) fn save_emulator_config_internal(
    platform: &str,
    exe_path: &str,
    launch_args_template: Option<String>,
    state: &AppState,
) -> Result<EmulatorConfig, String> {
    validate_platform(platform)?;
    let normalized_path = exe_path.trim();
    if normalized_path.is_empty() {
        return Err("Emulator executable path is required.".to_string());
    }

    if let Some(profile) = setup_profiles::get_default_platform_setup_profile(platform) {
        let file_name = Path::new(normalized_path)
            .file_name()
            .and_then(|f| f.to_str())
            .ok_or_else(|| "Invalid emulator path: missing file name.".to_string())?;

        let candidates = &profile.emulator.executable_candidates;
        if !candidates.is_empty() && !candidates.iter().any(|c| c.eq_ignore_ascii_case(file_name)) {
            return Err(format!(
                "Selected file '{}' does not match expected emulator executable for {} (expected: {})",
                file_name,
                profile.display_name,
                candidates.join(", ")
            ));
        }
    }

    let canon = crate::launcher::canonical_emulator_path(normalized_path)?;
    crate::orchestrator::record_exe(state, &canon)?;
    let canon_str = canon.to_string_lossy().to_string();

    let status = validate_emulator_status(Some(&canon_str));
    let template = normalize_launch_args_template(launch_args_template)
        .or_else(|| setup_profiles::default_launch_args_for(platform));

    lock_app_store(state)?.upsert_emulator_config(
        platform,
        Some(&canon_str),
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
