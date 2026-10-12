#[tauri::command]
pub fn check_requirements(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<RequirementsReport, String> {
    let content_dir = library_root_for_app_state(&state);
    let store = lock_store(&state)?;
    let game = store
        .get_game(&game_id)?
        .ok_or_else(|| format!("Unknown game: {game_id}"))?;
    build_requirements_report(&store, &content_dir, &game)
}

#[tauri::command]
pub fn get_library_statuses(state: State<'_, AppState>) -> Result<Vec<LibraryGameStatus>, String> {
    let content_dir = library_root_for_app_state(&state);
    let store = lock_store(&state)?;
    store
        .get_catalog()?
        .iter()
        .map(|game| build_library_status(&store, &content_dir, game))
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
    let content_dir = library_root_for_app_state(&state);
    let store = lock_store(&state)?;
    let game = store
        .get_game(&game_id)?
        .ok_or_else(|| format!("Unknown game: {game_id}"))?;
    build_game_setup_state(&store, &content_dir, &game)
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
    let content_dir = library_root_for_app_state(&state);
    let store = lock_store(&state)?;
    remove_profile_emulator_from_store(&store, &content_dir, profile_id.trim())
}

pub(super) fn remove_profile_emulator_from_store(
    store: &RepositoryStore,
    content_dir: &Path,
    profile_id: &str,
) -> Result<ProfileEmulatorRemovalReport, String> {
    let profile = setup_profiles::get_platform_setup_profile(profile_id)
        .ok_or_else(|| format!("Unknown setup profile: {profile_id}"))?;
    let mut deleted_files = false;
    let mut removed_path = None;

    if profile.emulator.install_mode == "downloadable" {
        let install_root = content_dir.join("Emulators");
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
pub async fn select_profile_emulator(
    profile_id: String,
    executable_path: String,
    state: State<'_, AppState>,
) -> Result<ProfileEmulatorConfig, String> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        select_profile_emulator_internal(profile_id.trim(), executable_path.trim(), &state)
    })
    .await
    .map_err(|e| format!("Select profile emulator task failed: {e}"))?
}

pub(crate) fn select_profile_emulator_internal(
    profile_id: &str,
    executable_path: &str,
    state: &AppState,
) -> Result<ProfileEmulatorConfig, String> {
    let profile = setup_profiles::get_platform_setup_profile(profile_id)
        .ok_or_else(|| format!("Unknown setup profile: {profile_id}"))?;
    let normalized_path = executable_path.trim();
    if normalized_path.is_empty() {
        return Err("Emulator executable path is required.".to_string());
    }

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

    let canon = crate::launcher::canonical_emulator_path(normalized_path)?;
    crate::orchestrator::record_exe(state, &canon)?;
    let canon_str = canon.to_string_lossy().to_string();

    let status = validate_emulator_status(Some(&canon_str));
    let store = lock_app_store(state)?;
    let config = store.upsert_profile_emulator_config(
        &profile.id,
        &profile.platform,
        Some(&canon_str),
        status,
        None,
        Some(&profile.launch.args_template),
    )?;
    Ok(config)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustUpdatedEmulatorReport {
    pub target: String,
    pub exe_path: String,
    pub previous_sha256_prefix: Option<String>,
    pub current_sha256_prefix: String,
    pub current_sha256: String,
    pub mtime_ms: u128,
}

#[tauri::command]
pub async fn trust_updated_emulator(
    target: String,
    state: State<'_, AppState>,
) -> Result<TrustUpdatedEmulatorReport, String> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        trust_updated_emulator_internal(target.trim(), &state)
    })
    .await
    .map_err(|e| format!("Trust updated emulator task failed: {e}"))?
}

pub(crate) fn trust_updated_emulator_internal(
    target: &str,
    state: &AppState,
) -> Result<TrustUpdatedEmulatorReport, String> {
    let store = lock_app_store(state)?;
    let trimmed = target.trim();
    let exe_path = if let Some(config) = store.get_profile_emulator_config(trimmed)? {
        config.exe_path
    } else if let Some(config) = store.get_emulator_config(trimmed)? {
        config.exe_path
    } else {
        None
    };

    let raw_path = exe_path.ok_or_else(|| format!("No emulator configured for {trimmed}"))?;
    let canon = crate::launcher::canonical_emulator_path(&raw_path)?;
    let key = crate::exe_integrity::record_key(&canon);

    let previous_sha256_prefix = store
        .get_config(&key)?
        .and_then(|raw| serde_json::from_str::<crate::exe_integrity::ExeRecord>(&raw).ok())
        .map(|rec| rec.sha256.chars().take(8).collect::<String>());

    let new_record = crate::exe_integrity::make_record(&canon)?;
    let current_sha256 = new_record.sha256.clone();
    let current_sha256_prefix = current_sha256.chars().take(8).collect::<String>();
    let mtime_ms = new_record.mtime_ms;

    let json = serde_json::to_string(&new_record).map_err(|e| e.to_string())?;
    store.set_config(&key, &json)?;

    Ok(TrustUpdatedEmulatorReport {
        target: trimmed.to_string(),
        exe_path: canon.to_string_lossy().to_string(),
        previous_sha256_prefix,
        current_sha256_prefix,
        current_sha256,
        mtime_ms,
    })
}

#[tauri::command]
pub fn import_profile_system_file(
    game_id: String,
    requirement_id: String,
    source_path: String,
    state: State<'_, AppState>,
) -> Result<ImportAssetFileReport, String> {
    let content_dir = library_root_for_app_state(&state);
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
        &content_dir,
        &profile,
        requirement,
        Path::new(source_path.trim()),
    )
}

#[tauri::command]
pub fn detect_system_file(
    source_path: String,
) -> Result<crate::system_files_detector::SystemFileDetection, String> {
    crate::system_files_detector::detect_system_file(Path::new(source_path.trim()))
}

#[tauri::command]
pub fn auto_import_system_file(
    source_path: String,
    state: State<'_, AppState>,
) -> Result<crate::system_files_detector::SmartImportReport, String> {
    let content_dir = library_root_for_app_state(&state);
    let store = lock_store(&state)?;
    crate::system_files_detector::auto_import_system_file_internal(
        &store,
        &content_dir,
        Path::new(source_path.trim()),
    )
}

#[tauri::command]
pub fn parse_deep_link(uri: String) -> Result<crate::deep_link::DeepLinkAction, String> {
    crate::deep_link::parse_deep_link(&uri)
}

#[tauri::command]
pub fn register_deep_link_protocol() -> Result<(), String> {
    crate::deep_link::register_windows_protocol()
}


