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
pub fn list_platform_setup_profiles() -> Result<Vec<PlatformSetupProfile>, String> {
    Ok(setup_profiles::list_platform_setup_profiles())
}

#[tauri::command]
pub fn get_game_setup_state(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<GameSetupState, String> {
    let store = lock_store(&state)?;
    let game = store
        .get_game(&game_id)?
        .ok_or_else(|| format!("Unknown game: {game_id}"))?;
    build_game_setup_state(&store, &state.data_dir, &game)
}

#[tauri::command]
pub async fn install_profile_emulator(
    profile_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ProfileEmulatorConfig, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
    let profile = setup_profiles::get_platform_setup_profile(profile_id.trim())
        .ok_or_else(|| format!("Unknown setup profile: {profile_id}"))?;
    if profile.emulator.install_mode != "downloadable" {
        return Err(format!(
            "{} requires manual emulator selection.",
            profile.display_name
        ));
    }

    crate::orchestrator::install_emulator_internal(
        &app,
        &state,
        &profile.platform,
        None,
        target_dir.as_deref(),
    )
    .await?;
    let store = lock_store(&state)?;
    store
        .get_profile_emulator_config(&profile.id)?
        .ok_or_else(|| format!("{} was installed but not persisted.", profile.display_name))
}

#[tauri::command]
pub fn remove_profile_emulator(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<ProfileEmulatorRemovalReport, String> {
    let store = lock_store(&state)?;
    remove_profile_emulator_from_store(&store, &state.data_dir, profile_id.trim())
}

pub(super) fn remove_profile_emulator_from_store(
    store: &RepositoryStore,
    data_dir: &Path,
    profile_id: &str,
) -> Result<ProfileEmulatorRemovalReport, String> {
    let profile = setup_profiles::get_platform_setup_profile(profile_id)
        .ok_or_else(|| format!("Unknown setup profile: {profile_id}"))?;
    let mut deleted_files = false;
    let mut removed_path = None;

    if profile.emulator.install_mode == "downloadable" {
        let install_root = data_dir.join("Emulators");
        let install_dir = install_root.join(&profile.platform).join(&profile.id);
        deleted_files = crate::archive::remove_directory_inside(&install_root, &install_dir)?;
        removed_path = Some(install_dir.to_string_lossy().to_string());
    }

    let removed_config = store.delete_profile_emulator_config(&profile.id)?;

    Ok(ProfileEmulatorRemovalReport {
        profile_id: profile.id,
        platform: profile.platform,
        removed_config,
        deleted_files,
        removed_path,
        message: None,
    })
}

#[tauri::command]
pub fn select_profile_emulator(
    profile_id: String,
    executable_path: String,
    state: State<'_, AppState>,
) -> Result<ProfileEmulatorConfig, String> {
    let profile = setup_profiles::get_platform_setup_profile(profile_id.trim())
        .ok_or_else(|| format!("Unknown setup profile: {profile_id}"))?;
    let normalized_path = executable_path.trim();
    if normalized_path.is_empty() {
        return Err("Emulator executable path is required.".to_string());
    }
    let status = validate_emulator_status(Some(normalized_path));
    let store = lock_store(&state)?;
    let config = store.upsert_profile_emulator_config(
        &profile.id,
        &profile.platform,
        Some(normalized_path),
        status,
        None,
        Some(&profile.launch.args_template),
    )?;
    Ok(config)
}

#[tauri::command]
pub fn import_profile_system_file(
    game_id: String,
    requirement_id: String,
    source_path: String,
    state: State<'_, AppState>,
) -> Result<ImportAssetFileReport, String> {
    let store = lock_store(&state)?;
    let game = store
        .get_game(&game_id)?
        .ok_or_else(|| format!("Unknown game: {game_id}"))?;
    let profile = resolve_known_profile(&game)
        .ok_or_else(|| format!("{} does not use a known setup profile.", game.title))?;
    let requirement = profile
        .system_files
        .iter()
        .find(|requirement| requirement.id == requirement_id)
        .ok_or_else(|| format!("Unknown profile system file: {requirement_id}"))?;

    import_profile_system_file_into_store(
        &store,
        &state.data_dir,
        &profile,
        requirement,
        Path::new(source_path.trim()),
    )
}
