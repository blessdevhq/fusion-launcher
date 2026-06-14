#[tauri::command]
pub async fn preview_repository(
    url: String,
    state: State<'_, AppState>,
) -> Result<RepositoryPreview, String> {
    let allow_dev_http = cfg!(debug_assertions);
    let repo = fetch_repository_schema(&url, allow_dev_http).await?;
    let preview = build_repository_preview(&url, &repo);
    logging::log_event(
        &state.data_dir,
        "repository_previewed",
        &[
            ("url", url.as_str()),
            ("repository_id", preview.id.as_str()),
        ],
    );
    Ok(preview)
}

#[tauri::command]
pub fn preview_repository_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<RepositoryPreview, String> {
    let (repo, url) = load_repository_schema_from_file(&path)?;
    let preview = build_repository_preview(&url, &repo);
    logging::log_event(
        &state.data_dir,
        "repository_previewed",
        &[
            ("url", url.as_str()),
            ("repository_id", preview.id.as_str()),
        ],
    );
    Ok(preview)
}

#[tauri::command]
pub fn preview_builtin_demo_repository(
    state: State<'_, AppState>,
) -> Result<RepositoryPreview, String> {
    let repo = load_builtin_demo_repository()?;
    let preview = build_repository_preview(builtin_demo::BUILTIN_DEMO_REPOSITORY_URL, &repo);
    logging::log_event(
        &state.data_dir,
        "repository_previewed",
        &[
            ("url", builtin_demo::BUILTIN_DEMO_REPOSITORY_URL),
            ("repository_id", preview.id.as_str()),
        ],
    );
    Ok(preview)
}

#[tauri::command]
pub async fn connect_repository(
    url: String,
    state: State<'_, AppState>,
) -> Result<RepositorySummary, String> {
    let allow_dev_http = cfg!(debug_assertions);
    let repo = fetch_repository_schema(&url, allow_dev_http).await?;

    let mut store = lock_store(&state)?;
    let summary = store.store_repository(&url, &repo)?;
    logging::log_event(
        &state.data_dir,
        "repository_connected",
        &[
            ("url", url.as_str()),
            ("repository_id", summary.id.as_str()),
        ],
    );
    Ok(summary)
}

#[tauri::command]
pub fn connect_repository_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<RepositorySummary, String> {
    let (repo, url) = load_repository_schema_from_file(&path)?;
    let mut store = lock_store(&state)?;
    let summary = store.store_repository(&url, &repo)?;
    logging::log_event(
        &state.data_dir,
        "repository_connected",
        &[
            ("url", url.as_str()),
            ("repository_id", summary.id.as_str()),
        ],
    );
    Ok(summary)
}

#[tauri::command]
pub fn connect_builtin_demo_repository(
    state: State<'_, AppState>,
) -> Result<RepositorySummary, String> {
    let repo = load_builtin_demo_repository()?;
    let mut store = lock_store(&state)?;
    let summary = store.store_repository(builtin_demo::BUILTIN_DEMO_REPOSITORY_URL, &repo)?;
    logging::log_event(
        &state.data_dir,
        "repository_connected",
        &[
            ("url", builtin_demo::BUILTIN_DEMO_REPOSITORY_URL),
            ("repository_id", summary.id.as_str()),
        ],
    );
    Ok(summary)
}

#[tauri::command]
pub fn repair_library(state: State<'_, AppState>) -> Result<RepairLibraryReport, String> {
    let mut store = lock_store(&state)?;
    repair_library_state(&mut store, &state.data_dir)
}

#[tauri::command]
pub async fn refresh_repository(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<RepositorySummary, String> {
    let url = {
        let store = lock_store(&state)?;
        store
            .get_repository_url(&repository_id)?
            .ok_or_else(|| format!("Unknown repository: {repository_id}"))?
    };
    let repo = if builtin_demo::is_builtin_repository_url(&url) {
        load_builtin_demo_repository()?
    } else if is_file_repository_url(&url) {
        load_repository_schema_from_file_url(&url)?.0
    } else {
        let allow_dev_http = cfg!(debug_assertions);
        fetch_repository_schema(&url, allow_dev_http).await?
    };
    let mut store = lock_store(&state)?;
    let summary = store.store_repository(&url, &repo)?;
    logging::log_event(
        &state.data_dir,
        "repository_refreshed",
        &[
            ("url", url.as_str()),
            ("repository_id", summary.id.as_str()),
        ],
    );
    Ok(summary)
}

#[tauri::command]
pub fn list_repositories(state: State<'_, AppState>) -> Result<Vec<RepositorySummary>, String> {
    lock_store(&state)?.list_repositories()
}

#[tauri::command]
pub fn get_onboarding_state(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    let store = lock_store(&state)?;
    let repositories = store.list_repositories()?;
    let catalog = store.get_catalog()?;
    let catalog_count = catalog.len();
    let valid_emulator_platforms = store
        .list_emulator_configs()?
        .into_iter()
        .filter(|config| setup_profiles::has_default_setup_profile(&config.platform))
        .filter(|config| config.status == "valid")
        .map(|config| config.platform)
        .collect::<HashSet<_>>();
    let valid_emulator_count = valid_emulator_platforms.len();
    let catalog_platforms = catalog
        .iter()
        .map(|game| game.platform.clone())
        .collect::<HashSet<_>>();
    let repositories_configured = !repositories.is_empty();
    let emulators_configured = if catalog_platforms.is_empty() {
        valid_emulator_count > 0
    } else {
        catalog_platforms
            .iter()
            .any(|platform| valid_emulator_platforms.contains(platform))
    };
    let step = if !repositories_configured {
        "addRepository"
    } else if !emulators_configured {
        "configureEmulator"
    } else {
        "complete"
    };

    Ok(OnboardingState {
        step: step.to_string(),
        repositories_configured,
        emulators_configured,
        catalog_count,
        valid_emulator_count,
    })
}

#[tauri::command]
pub fn disconnect_repository(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    lock_store(&state)?.disconnect_repository(&repository_id)
}

#[tauri::command]
pub fn get_catalog(state: State<'_, AppState>) -> Result<Vec<CatalogGameView>, String> {
    lock_store(&state)?.get_catalog()
}

#[tauri::command]
pub fn get_game(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Option<CatalogGameView>, String> {
    lock_store(&state)?.get_game(&game_id)
}
