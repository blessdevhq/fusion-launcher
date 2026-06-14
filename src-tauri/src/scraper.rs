use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter};

use crate::game_files;
use crate::schema::{
    CatalogGameView, GameMetadata, LibraryScrapeStatus, ManualGameMetadataInput, ScrapeCandidate,
    ScrapeStateView, ScrapedGamePayload, ScreenScraperStatus, SteamGridDbStatus,
};
use crate::scrapers::screenscraper::{
    system_id_for_platform, Credentials, RequestOptions, ScreenScraperClient, PROVIDER,
};
use crate::scrapers::steamgriddb::{
    self, ApiKeySource, ResolvedApiKey, SteamGridDbClient, DAILY_REQUEST_LIMIT as SGDB_DAILY_LIMIT,
    PROVIDER as STEAMGRIDDB_PROVIDER,
};
use crate::setup_profiles;
use crate::storage::{metadata_name_key, steamgriddb_cache_key, MetadataCacheEntry};
use crate::AppState;

const DAILY_REQUEST_LIMIT: u32 = 20_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataEvent {
    game_id: String,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HashProgressEvent {
    game_id: String,
    read_bytes: u64,
    total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchProgressEvent {
    done: usize,
    total: usize,
    current_game_id: Option<String>,
}

#[derive(Clone)]
pub struct LibraryScrapeRuntime {
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl LibraryScrapeRuntime {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            handle: Arc::new(Mutex::new(None)),
        }
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn start(&self, handle: JoinHandle<()>) {
        self.cancel.store(false, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
        if let Ok(mut current) = self.handle.lock() {
            *current = Some(handle);
        }
    }

    fn finish(&self) {
        self.cancel.store(false, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut handle) = self.handle.lock() {
            *handle = None;
        }
    }
}

impl Default for LibraryScrapeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn scrape_game(app: AppHandle, state: AppState, game_id: String) -> Result<(), String> {
    let game = load_game(&state, &game_id)?;
    let system_id = match system_id_for_platform(&game.platform) {
        Some(system_id) => system_id,
        None => {
            set_state(
                &state,
                &app,
                &game.id,
                "skipped",
                None,
                &[],
                Some("ScreenScraper system mapping is not configured for this platform."),
            )?;
            return Ok(());
        }
    };

    // Check credentials before hashing: a missing account short-circuits to
    // "skipped" without reading the (potentially multi-GB) ROM for nothing.
    let credentials = match load_credentials(&state)? {
        Some(credentials) => credentials,
        None => {
            set_state(
                &state,
                &app,
                &game.id,
                "skipped",
                None,
                &[],
                Some("ScreenScraper credentials are not configured."),
            )?;
            return Err("ScreenScraper credentials are not configured.".to_string());
        }
    };
    let options = load_options(&state)?;
    let client = ScreenScraperClient::new(credentials, options)?;

    let mut hash_cache_key = None;
    if !should_skip_hash(&game) {
        if let Some(path) = installed_game_path(&state, &game)? {
            if hash_excluded_extension(&path) {
                set_state(
                    &state,
                    &app,
                    &game.id,
                    "fetching",
                    Some("name"),
                    &[],
                    Some("This file type is matched by name only."),
                )?;
            } else if let Some(entry) = hash_and_check_cache(&app, &state, &game, &path).await? {
                mark_ready_from_cache(&state, &app, &game.id, &entry)?;
                let _ = enrich_artwork_steamgriddb(&app, &state, &game).await;
                return Ok(());
            } else {
                hash_cache_key = load_crc32(&state, &game.id)?;
            }
        }
    }

    if let Some(crc32) = hash_cache_key.as_deref() {
        set_state(&state, &app, &game.id, "fetching", Some("hash"), &[], None)?;
        consume_request(&state)?;
        if let Some(payload) = client.by_hash(system_id, crc32).await? {
            persist_ready(&state, &app, &game.id, crc32, &payload, "hash")?;
            let _ = enrich_artwork_steamgriddb(&app, &state, &game).await;
            return Ok(());
        }
    }

    scrape_by_name(app, state, game, system_id, client, hash_cache_key).await
}

pub async fn apply_override(
    app: AppHandle,
    state: AppState,
    game_id: String,
    provider_game_id: String,
) -> Result<(), String> {
    let game = load_game(&state, &game_id)?;
    let credentials = load_credentials(&state)?
        .ok_or_else(|| "ScreenScraper credentials are not configured.".to_string())?;
    let options = load_options(&state)?;
    let client = ScreenScraperClient::new(credentials, options)?;
    consume_request(&state)?;
    let payload = client
        .by_id(&provider_game_id)
        .await?
        .ok_or_else(|| format!("ScreenScraper game was not found: {provider_game_id}"))?;

    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.put_override(&game_id, &provider_game_id, &payload)?;
        store.set_scrape_state(
            &game_id,
            "ready",
            Some("override"),
            &[],
            Some("Manual ScreenScraper override applied."),
        )?;
    }
    emit_metadata(&app, "metadata:ready", &game_id, "ready");
    let _ = enrich_artwork_steamgriddb(&app, &state, &game).await;
    Ok(())
}

pub fn save_manual_metadata(
    app: &AppHandle,
    state: &AppState,
    game_id: &str,
    input: ManualGameMetadataInput,
) -> Result<(), String> {
    let payload = build_manual_payload(input);
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store
            .get_game(game_id)?
            .ok_or_else(|| format!("Unknown game: {game_id}"))?;
        store.put_override(game_id, "manual", &payload)?;
        store.set_scrape_state(
            game_id,
            "ready",
            Some("override"),
            &[],
            Some("Manual metadata override saved."),
        )?;
    }
    emit_metadata(app, "metadata:ready", game_id, "ready");
    Ok(())
}

pub fn clear_override(app: &AppHandle, state: &AppState, game_id: &str) -> Result<bool, String> {
    let changed = {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        let changed = store.clear_override(game_id)?;
        store.set_scrape_state(
            game_id,
            "pending",
            None,
            &[],
            Some("Manual ScreenScraper override cleared."),
        )?;
        changed
    };
    emit_metadata(app, "metadata:ready", game_id, "pending");
    Ok(changed)
}

fn build_manual_payload(input: ManualGameMetadataInput) -> ScrapedGamePayload {
    ScrapedGamePayload {
        provider: "manual".to_string(),
        provider_game_id: Some("manual".to_string()),
        title: trim_optional(input.title),
        description: trim_optional(input.description),
        cover: trim_optional(input.cover),
        hero: trim_optional(input.hero),
        logo: trim_optional(input.logo),
        screenshots: input
            .screenshots
            .into_iter()
            .filter_map(|value| trim_optional(Some(value)))
            .collect(),
        metadata: sanitize_manual_metadata(input.metadata),
    }
}

fn sanitize_manual_metadata(metadata: Option<GameMetadata>) -> GameMetadata {
    let Some(metadata) = metadata else {
        return empty_metadata();
    };
    GameMetadata {
        release_year: metadata.release_year,
        developer: trim_optional(metadata.developer),
        publisher: trim_optional(metadata.publisher),
        genres: trim_string_list(metadata.genres),
        tags: trim_string_list(metadata.tags),
        players: trim_optional(metadata.players),
        series: trim_optional(metadata.series),
        external_ids: metadata
            .external_ids
            .into_iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                let value = value.trim();
                if key.is_empty() || value.is_empty() {
                    None
                } else {
                    Some((key.to_string(), value.to_string()))
                }
            })
            .collect(),
    }
}

fn empty_metadata() -> GameMetadata {
    GameMetadata {
        release_year: None,
        developer: None,
        publisher: None,
        genres: Vec::new(),
        tags: Vec::new(),
        players: None,
        series: None,
        external_ids: std::collections::BTreeMap::new(),
    }
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn trim_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|value| trim_optional(Some(value)))
        .collect()
}

pub fn default_state(game_id: &str) -> ScrapeStateView {
    ScrapeStateView {
        game_id: game_id.to_string(),
        status: "pending".to_string(),
        match_kind: None,
        candidates: Vec::new(),
        message: None,
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

pub fn get_status(state: &AppState) -> Result<ScreenScraperStatus, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    let ssid = store
        .get_config("screenscraper.ssid")?
        .filter(|value| !value.trim().is_empty());
    let configured = ssid.is_some()
        && store
            .get_config("screenscraper.sspassword")?
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    Ok(ScreenScraperStatus {
        configured,
        ssid,
        region: normalize_region(store.get_config("scrape.region")?.as_deref()),
        daily_requests: store.get_screenscraper_request_count()?,
        daily_limit: DAILY_REQUEST_LIMIT,
    })
}

pub fn save_credentials(
    state: &AppState,
    ssid: &str,
    sspassword: &str,
    region: Option<&str>,
) -> Result<ScreenScraperStatus, String> {
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.set_config("screenscraper.ssid", ssid.trim())?;
        if !sspassword.trim().is_empty() {
            store.set_config("screenscraper.sspassword", sspassword.trim())?;
        } else if store.get_config("screenscraper.sspassword")?.is_none() {
            store.set_config("screenscraper.sspassword", "")?;
        }
        store.set_config("scrape.region", &normalize_region(region))?;
    }

    get_status(state)
}

pub fn get_steamgriddb_status(state: &AppState) -> Result<SteamGridDbStatus, String> {
    let (resolved, daily_requests, pending_batch) = {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        let user_key = store.get_config("steamgriddb.api_key")?;
        (
            steamgriddb::resolve_api_key(user_key.as_deref()),
            store.get_steamgriddb_request_count()?,
            store.count_pending_scrape_states()?,
        )
    };
    let key_source = match resolved.as_ref().map(|value| &value.source) {
        Some(ApiKeySource::User) => "user",
        Some(ApiKeySource::BuiltIn) => "built-in",
        None => "none",
    };

    Ok(SteamGridDbStatus {
        configured: resolved.is_some(),
        key_source: key_source.to_string(),
        daily_requests,
        daily_limit: SGDB_DAILY_LIMIT,
        pending_batch,
        batch_running: state.library_scrape.is_running(),
    })
}

pub fn save_steamgriddb_key(state: &AppState, api_key: &str) -> Result<SteamGridDbStatus, String> {
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.set_config("steamgriddb.api_key", api_key.trim())?;
    }
    get_steamgriddb_status(state)
}

pub fn scrape_library(app: AppHandle, state: AppState) -> Result<LibraryScrapeStatus, String> {
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.mark_installed_games_pending_for_scrape()?;
    }
    start_library_scrape_worker(app, state.clone());
    get_library_scrape_status(&state)
}

pub fn cancel_library_scrape(state: &AppState) -> Result<LibraryScrapeStatus, String> {
    state.library_scrape.cancel();
    get_library_scrape_status(state)
}

pub fn get_library_scrape_status(state: &AppState) -> Result<LibraryScrapeStatus, String> {
    let pending = {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.count_pending_scrape_states()?
    };
    Ok(LibraryScrapeStatus {
        running: state.library_scrape.is_running(),
        pending,
    })
}

async fn scrape_by_name(
    app: AppHandle,
    state: AppState,
    game: CatalogGameView,
    system_id: u32,
    client: ScreenScraperClient,
    hash_cache_key: Option<String>,
) -> Result<(), String> {
    let name_cache_key = metadata_name_key(&game.platform, &game.title);
    if let Some(entry) = cache_by_key(&state, &name_cache_key)? {
        mark_ready_from_cache(&state, &app, &game.id, &entry)?;
        let _ = enrich_artwork_steamgriddb(&app, &state, &game).await;
        return Ok(());
    }

    set_state(&state, &app, &game.id, "fetching", Some("name"), &[], None)?;
    consume_request(&state)?;
    let candidates = client.by_name(system_id, &game.title).await?;
    if candidates.is_empty() {
        set_state(
            &state,
            &app,
            &game.id,
            "failed",
            Some("name"),
            &[],
            Some("ScreenScraper did not return a match."),
        )?;
        return Ok(());
    }
    if candidates.len() > 1 {
        set_state(
            &state,
            &app,
            &game.id,
            "ambiguous",
            Some("name"),
            &candidates,
            Some("Multiple ScreenScraper candidates were found."),
        )?;
        emit_metadata(&app, "metadata:ambiguous", &game.id, "ambiguous");
        return Ok(());
    }

    let candidate = &candidates[0];
    consume_request(&state)?;
    let payload = client
        .by_id(&candidate.provider_game_id)
        .await?
        .unwrap_or_else(|| payload_from_candidate(candidate));
    let cache_key = hash_cache_key.as_deref().unwrap_or(&name_cache_key);
    persist_ready(&state, &app, &game.id, cache_key, &payload, "name")?;
    let _ = enrich_artwork_steamgriddb(&app, &state, &game).await;
    Ok(())
}

fn start_library_scrape_worker(app: AppHandle, state: AppState) {
    if state.library_scrape.is_running() {
        return;
    }

    let runtime = state.library_scrape.clone();
    let runtime_for_task = runtime.clone();
    let handle = tauri::async_runtime::spawn(async move {
        drain_library_scrape_queue(app, state.clone(), runtime_for_task.clone()).await;
        runtime_for_task.finish();
    });
    runtime.start(handle);
}

async fn drain_library_scrape_queue(
    app: AppHandle,
    state: AppState,
    runtime: LibraryScrapeRuntime,
) {
    let total = pending_scrape_ids(&state).map(|ids| ids.len()).unwrap_or(0);
    let mut done = 0usize;
    emit_batch_progress(&app, done, total, None);

    loop {
        if runtime.cancelled() {
            break;
        }
        let Some(game_id) = pending_scrape_ids(&state)
            .ok()
            .and_then(|ids| ids.into_iter().next())
        else {
            break;
        };

        emit_batch_progress(&app, done, total, Some(game_id.clone()));
        let _ = scrape_game(app.clone(), state.clone(), game_id).await;
        done += 1;
        emit_batch_progress(&app, done, total, None);
    }
}

fn pending_scrape_ids(state: &AppState) -> Result<Vec<String>, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    store.list_pending_scrape_game_ids()
}

async fn enrich_artwork_steamgriddb(
    app: &AppHandle,
    state: &AppState,
    game: &CatalogGameView,
) -> Result<bool, String> {
    // Skip games that already have fresh SteamGridDB artwork: re-scrapes, batch
    // re-runs and the ScreenScraper cache-hit path would otherwise burn the daily
    // request budget on art we already cached (TTL on metadata_cache drives refresh).
    if steamgriddb_cache_is_fresh(state, &game.id)? {
        return Ok(false);
    }
    let Some(resolved_key) = load_steamgriddb_key(state)? else {
        return Ok(false);
    };
    let client = SteamGridDbClient::new(resolved_key.key)?;
    let Some(sgdb_game) = find_steamgriddb_game(state, &client, game).await? else {
        return Ok(false);
    };

    let hero = fetch_steamgriddb_asset(state, || client.best_hero(sgdb_game.id)).await;
    let logo = fetch_steamgriddb_asset(state, || client.best_logo(sgdb_game.id)).await;
    let grid = fetch_steamgriddb_asset(state, || client.best_grid(sgdb_game.id)).await;
    if hero.is_none() && logo.is_none() && grid.is_none() {
        return Ok(false);
    }

    let payload = steamgriddb::payload_from_artwork(&sgdb_game, hero, logo, grid);
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.put_metadata(
            &steamgriddb_cache_key(&game.id),
            STEAMGRIDDB_PROVIDER,
            &payload,
            "name",
        )?;
        if let Some(current) = store.get_scrape_state(&game.id)? {
            store.set_scrape_state(
                &game.id,
                &current.status,
                current.match_kind.as_deref(),
                &current.candidates,
                Some("SteamGridDB hero/logo added."),
            )?;
        }
    }
    emit_metadata(app, "metadata:ready", &game.id, "ready");
    Ok(true)
}

async fn find_steamgriddb_game(
    state: &AppState,
    client: &SteamGridDbClient,
    game: &CatalogGameView,
) -> Result<Option<steamgriddb::SteamGridDbGame>, String> {
    for query in steamgriddb_queries(game) {
        consume_steamgriddb_request(state)?;
        let games = client.autocomplete(&query).await?;
        if let Some(match_) = steamgriddb::best_game_for_query(&games, &query) {
            return Ok(Some(match_));
        }
    }
    Ok(None)
}

async fn fetch_steamgriddb_asset<F, Fut>(state: &AppState, fetch: F) -> Option<String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<String>, String>>,
{
    consume_steamgriddb_request(state).ok()?;
    fetch().await.ok().flatten()
}

fn steamgriddb_queries(game: &CatalogGameView) -> Vec<String> {
    let mut queries = vec![game.title.clone()];
    if let Some(series) = game
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.series.as_deref())
        .filter(|series| !series.trim().is_empty())
    {
        let series_query = format!("{} {}", series.trim(), game.title.trim());
        if !series_query.eq_ignore_ascii_case(&game.title) {
            queries.push(series_query);
        }
    }
    queries
}

async fn hash_and_check_cache(
    app: &AppHandle,
    state: &AppState,
    game: &CatalogGameView,
    path: &Path,
) -> Result<Option<MetadataCacheEntry>, String> {
    set_state(state, app, &game.id, "hashing", Some("hash"), &[], None)?;
    let app_for_hash = app.clone();
    let game_id = game.id.clone();
    let path = path.to_path_buf();
    let hashes = tokio::task::spawn_blocking(move || {
        crate::rom_hasher::hash_rom(&path, |read_bytes, total_bytes| {
            let _ = app_for_hash.emit(
                "hash:progress",
                HashProgressEvent {
                    game_id: game_id.clone(),
                    read_bytes,
                    total_bytes,
                },
            );
        })
    })
    .await
    .map_err(|error| format!("ROM hashing task failed: {error}"))??;

    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.upsert_rom_hashes(&game.id, &hashes)?;
        store.get_metadata_by_key(&hashes.crc32, PROVIDER)
    }
}

fn load_game(state: &AppState, game_id: &str) -> Result<CatalogGameView, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    store
        .get_game(game_id)?
        .ok_or_else(|| format!("Unknown game: {game_id}"))
}

fn load_crc32(state: &AppState, game_id: &str) -> Result<Option<String>, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    store.get_rom_crc32(game_id)
}

fn cache_by_key(state: &AppState, key: &str) -> Result<Option<MetadataCacheEntry>, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    store.get_metadata_by_key(key, PROVIDER)
}

fn persist_ready(
    state: &AppState,
    app: &AppHandle,
    game_id: &str,
    cache_key: &str,
    payload: &ScrapedGamePayload,
    match_kind: &str,
) -> Result<(), String> {
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.put_metadata(cache_key, PROVIDER, payload, match_kind)?;
        store.set_scrape_state(game_id, "ready", Some(match_kind), &[], None)?;
    }
    emit_metadata(app, "metadata:ready", game_id, "ready");
    Ok(())
}

fn mark_ready_from_cache(
    state: &AppState,
    app: &AppHandle,
    game_id: &str,
    entry: &MetadataCacheEntry,
) -> Result<(), String> {
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.set_scrape_state(game_id, "ready", Some(&entry.match_kind), &[], None)?;
    }
    emit_metadata(app, "metadata:ready", game_id, "ready");
    Ok(())
}

fn set_state(
    state: &AppState,
    app: &AppHandle,
    game_id: &str,
    status: &str,
    match_kind: Option<&str>,
    candidates: &[ScrapeCandidate],
    message: Option<&str>,
) -> Result<ScrapeStateView, String> {
    let next = {
        let store = state
            .store
            .lock()
            .map_err(|_| "Store lock poisoned.".to_string())?;
        store.set_scrape_state(game_id, status, match_kind, candidates, message)?
    };
    emit_metadata(app, "metadata:state", game_id, status);
    Ok(next)
}

fn installed_game_path(
    state: &AppState,
    game: &CatalogGameView,
) -> Result<Option<PathBuf>, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    let raw_path = store
        .get_download(&game.id)?
        .and_then(|download| download.local_path)
        .or_else(|| {
            store
                .get_torrent_download(&game.id)
                .ok()
                .flatten()
                .filter(|download| download.status == "completed")
                .map(|download| download.save_dir)
        });
    drop(store);

    let Some(raw_path) = raw_path else {
        return Ok(None);
    };
    if game.expected_extensions.is_empty() {
        return Ok(Some(PathBuf::from(raw_path)));
    }
    let extensions = game_files::normalize_expected_extensions(&game.expected_extensions)?;
    let preferred_file = game
        .launch
        .as_ref()
        .and_then(|launch| launch.preferred_file.as_deref());
    match game_files::resolve_game_path(Path::new(&raw_path), &extensions, preferred_file) {
        Ok(path) => Ok(Some(path)),
        Err(_) => Ok(Some(PathBuf::from(raw_path))),
    }
}

fn should_skip_hash(game: &CatalogGameView) -> bool {
    if game.content_mode.as_deref() == Some("metadata_only") {
        return true;
    }
    if known_profile_allows_directory(game) {
        return true;
    }
    game.expected_extensions
        .iter()
        .any(|extension| hash_excluded_extension_str(extension))
}

fn known_profile_allows_directory(game: &CatalogGameView) -> bool {
    game.setup_profile_id
        .as_deref()
        .and_then(setup_profiles::get_platform_setup_profile)
        .or_else(|| setup_profiles::get_default_platform_setup_profile(&game.platform))
        .map(|profile| profile.game_files.allow_directory)
        .unwrap_or(false)
}

fn hash_excluded_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{}", extension.to_lowercase()))
        .map(|extension| hash_excluded_extension_str(&extension))
        .unwrap_or(false)
}

fn hash_excluded_extension_str(extension: &str) -> bool {
    matches!(
        extension.to_lowercase().as_str(),
        ".chd" | ".cso" | ".cue" | ".gdi" | ".m3u" | ".pbp" | ".rvz"
    )
}

fn load_credentials(state: &AppState) -> Result<Option<Credentials>, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    let ssid = store
        .get_config("screenscraper.ssid")?
        .unwrap_or_default()
        .trim()
        .to_string();
    let sspassword = store
        .get_config("screenscraper.sspassword")?
        .unwrap_or_default()
        .trim()
        .to_string();
    if ssid.is_empty() || sspassword.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Credentials { ssid, sspassword }))
    }
}

fn steamgriddb_cache_is_fresh(state: &AppState, game_id: &str) -> Result<bool, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    Ok(store
        .get_metadata_by_key(&steamgriddb_cache_key(game_id), STEAMGRIDDB_PROVIDER)?
        .is_some())
}

fn load_steamgriddb_key(state: &AppState) -> Result<Option<ResolvedApiKey>, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    let user_key = store.get_config("steamgriddb.api_key")?;
    Ok(steamgriddb::resolve_api_key(user_key.as_deref()))
}

fn load_options(state: &AppState) -> Result<RequestOptions, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    Ok(RequestOptions {
        language: store
            .get_config("scrape.language")?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "en".to_string()),
        region: normalize_region(store.get_config("scrape.region")?.as_deref()),
    })
}

fn normalize_region(region: Option<&str>) -> String {
    match region.unwrap_or("auto").trim().to_lowercase().as_str() {
        "eu" => "eu".to_string(),
        "us" => "us".to_string(),
        "jp" => "jp".to_string(),
        _ => "auto".to_string(),
    }
}

fn consume_request(state: &AppState) -> Result<(), String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    store
        .consume_screenscraper_request(DAILY_REQUEST_LIMIT)
        .map(|_| ())
}

fn consume_steamgriddb_request(state: &AppState) -> Result<(), String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned.".to_string())?;
    store
        .consume_steamgriddb_request(SGDB_DAILY_LIMIT)
        .map(|_| ())
}

fn payload_from_candidate(candidate: &ScrapeCandidate) -> ScrapedGamePayload {
    let mut external_ids = std::collections::BTreeMap::new();
    external_ids.insert(PROVIDER.to_string(), candidate.provider_game_id.clone());
    ScrapedGamePayload {
        provider: PROVIDER.to_string(),
        provider_game_id: Some(candidate.provider_game_id.clone()),
        title: Some(candidate.title.clone()),
        description: None,
        cover: candidate.cover.clone(),
        hero: None,
        logo: None,
        screenshots: Vec::new(),
        metadata: crate::schema::GameMetadata {
            release_year: candidate.release_year,
            developer: candidate.developer.clone(),
            publisher: None,
            genres: Vec::new(),
            tags: vec!["name".to_string()],
            players: None,
            series: None,
            external_ids,
        },
    }
}

fn emit_metadata(app: &AppHandle, event: &str, game_id: &str, status: &str) {
    let _ = app.emit(
        event,
        MetadataEvent {
            game_id: game_id.to_string(),
            status: status.to_string(),
        },
    );
}

fn emit_batch_progress(
    app: &AppHandle,
    done: usize,
    total: usize,
    current_game_id: Option<String>,
) {
    let _ = app.emit(
        "scrape:batch",
        BatchProgressEvent {
            done,
            total,
            current_game_id,
        },
    );
}
