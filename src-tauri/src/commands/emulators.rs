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
        .or_else(|| setup_profiles::default_launch_args_for(&platform));

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
