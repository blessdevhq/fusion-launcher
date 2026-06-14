#[tauri::command]
pub async fn scrape_game(
    game_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    crate::scraper::scrape_game(app, state.inner().clone(), game_id).await
}

#[tauri::command]
pub fn get_scrape_state(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<ScrapeStateView, String> {
    let store = lock_store(&state)?;
    Ok(store
        .get_scrape_state(&game_id)?
        .unwrap_or_else(|| crate::scraper::default_state(&game_id)))
}

#[tauri::command]
pub fn list_scrape_candidates(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ScrapeCandidate>, String> {
    let store = lock_store(&state)?;
    Ok(store
        .get_scrape_state(&game_id)?
        .map(|state| state.candidates)
        .unwrap_or_default())
}

#[tauri::command]
pub async fn apply_scrape_override(
    game_id: String,
    provider_game_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    crate::scraper::apply_override(app, state.inner().clone(), game_id, provider_game_id).await
}

#[tauri::command]
pub fn save_manual_metadata(
    game_id: String,
    metadata: ManualGameMetadataInput,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    crate::scraper::save_manual_metadata(&app, state.inner(), &game_id, metadata)
}

#[tauri::command]
pub fn clear_scrape_override(
    game_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<bool, String> {
    crate::scraper::clear_override(&app, state.inner(), &game_id)
}

#[tauri::command]
pub fn save_screenscraper_credentials(
    ssid: String,
    sspassword: String,
    region: Option<String>,
    state: State<'_, AppState>,
) -> Result<ScreenScraperStatus, String> {
    crate::scraper::save_credentials(state.inner(), &ssid, &sspassword, region.as_deref())
}

#[tauri::command]
pub fn get_screenscraper_status(state: State<'_, AppState>) -> Result<ScreenScraperStatus, String> {
    crate::scraper::get_status(state.inner())
}

#[tauri::command]
pub fn save_steamgriddb_key(
    api_key: String,
    state: State<'_, AppState>,
) -> Result<SteamGridDbStatus, String> {
    crate::scraper::save_steamgriddb_key(state.inner(), &api_key)
}

#[tauri::command]
pub fn get_steamgriddb_status(state: State<'_, AppState>) -> Result<SteamGridDbStatus, String> {
    crate::scraper::get_steamgriddb_status(state.inner())
}

#[tauri::command]
pub fn scrape_library(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<LibraryScrapeStatus, String> {
    crate::scraper::scrape_library(app, state.inner().clone())
}

#[tauri::command]
pub fn cancel_library_scrape(state: State<'_, AppState>) -> Result<LibraryScrapeStatus, String> {
    crate::scraper::cancel_library_scrape(state.inner())
}
