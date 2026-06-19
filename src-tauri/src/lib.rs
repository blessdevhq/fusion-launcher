use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::Manager;

mod app_update;
mod archive;
mod builtin_demo;
mod commands;
mod downloads;
mod emulator_profiles;
mod game_files;
mod github_resolver;
mod launcher;
mod libtorrent_engine;
mod logging;
mod manifest;
mod net;
mod orchestrator;
mod rom_hasher;
mod schema;
mod scraper;
mod scrapers;
mod security;
mod setup_profiles;
mod storage;
mod torrent;
mod torrent_engine;

use storage::RepositoryStore;
use torrent::TorrentManager;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<RepositoryStore>>,
    /// Roaming AppData: SQLite DB, logs, and config live here.
    pub data_dir: PathBuf,
    /// Default root for large managed content (Emulators/Games/System/Temp),
    /// `AppData\Local` unless overridden by the `library_root` config key.
    pub local_data_dir: PathBuf,
    /// `None` when the torrent engine failed to start this session. Direct HTTP
    /// downloads keep working.
    pub torrents: Option<TorrentManager>,
    pub running_games: Arc<Mutex<HashMap<String, u32>>>,
    pub library_scrape: scraper::LibraryScrapeRuntime,
}

impl AppState {
    /// Returns the torrent engine, or a user-facing error if it could not be
    /// initialized this session. Callers that need magnet/torrent support use
    /// `state.torrents()?`; everything else stays unaffected.
    pub fn torrents(&self) -> Result<&TorrentManager, String> {
        self.torrents.as_ref().ok_or_else(|| {
            "Torrent engine is unavailable for this session, so magnet downloads are disabled. \
             Restart Fusion Launcher to try again."
                .to_string()
        })
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| setup_error(error.to_string()))?;
            std::fs::create_dir_all(&data_dir).map_err(|error| setup_error(error.to_string()))?;
            let local_data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|error| setup_error(error.to_string()))?;
            std::fs::create_dir_all(&local_data_dir)
                .map_err(|error| setup_error(error.to_string()))?;
            let database_path = prepare_database_path(&data_dir).map_err(setup_error)?;
            logging::initialize(&data_dir);
            let mut repository_store =
                RepositoryStore::open(&database_path).map_err(setup_error)?;
            // Large content lives under the library root: the `library_root`
            // config override if set, else `AppData\Local`. The DB/logs/config
            // stay in Roaming (`data_dir`).
            let library_root = repository_store
                .get_config("library_root")
                .ok()
                .flatten()
                .map(PathBuf::from)
                .unwrap_or_else(|| local_data_dir.clone());
            match commands::repair_library_state(&mut repository_store, &library_root) {
                Ok(report) if report.repaired => {
                    logging::log_event(
                        &data_dir,
                        "library_repaired",
                        &[(
                            "repository_id",
                            report.repository_id.as_deref().unwrap_or(""),
                        )],
                    );
                }
                Ok(_) => {}
                Err(message) => {
                    logging::log_event(
                        &data_dir,
                        "library_repair_failed",
                        &[("message", message.as_str())],
                    );
                }
            }
            let store = Arc::new(Mutex::new(repository_store));
            // Torrent startup is best-effort: a failure here must not stop the
            // launcher from opening. The libtorrent sidecar is spawned lazily per
            // download, so this rarely fails; direct HTTP downloads and the rest
            // of the app work regardless.
            let torrents = match tauri::async_runtime::block_on(TorrentManager::new(
                library_root.clone(),
                Arc::clone(&store),
                app.handle().clone(),
            )) {
                Ok(manager) => Some(manager),
                Err(message) => {
                    logging::log_event(
                        &data_dir,
                        "torrent_init_failed",
                        &[("message", message.as_str())],
                    );
                    None
                }
            };

            let app_state = AppState {
                store,
                data_dir,
                local_data_dir,
                torrents,
                running_games: Arc::new(Mutex::new(HashMap::new())),
                library_scrape: scraper::LibraryScrapeRuntime::new(),
            };
            app.manage(app_state.clone());

            // Background cover backfill: fill in missing cover art for catalog
            // games on launch. No-op without a SteamGridDB key, and capped so a
            // large catalog never triggers a giant scrape.
            scraper::auto_scrape_missing_artwork(app.handle().clone(), app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::preview_repository,
            commands::preview_repository_file,
            commands::preview_builtin_demo_repository,
            commands::preview_source,
            commands::add_source,
            commands::connect_repository,
            commands::connect_repository_file,
            commands::connect_builtin_demo_repository,
            commands::repair_library,
            commands::refresh_repository,
            commands::get_onboarding_state,
            commands::list_repositories,
            commands::disconnect_repository,
            commands::get_catalog,
            commands::get_game,
            commands::scrape_game,
            commands::get_scrape_state,
            commands::list_scrape_candidates,
            commands::apply_scrape_override,
            commands::save_manual_metadata,
            commands::clear_scrape_override,
            commands::save_screenscraper_credentials,
            commands::get_screenscraper_status,
            commands::save_steamgriddb_key,
            commands::get_steamgriddb_status,
            commands::scrape_library,
            commands::cancel_library_scrape,
            commands::check_requirements,
            commands::get_library_statuses,
            commands::list_platform_setup_profiles,
            commands::get_game_setup_state,
            commands::install_profile_emulator,
            commands::remove_profile_emulator,
            commands::select_profile_emulator,
            commands::import_profile_system_file,
            commands::list_emulator_configs,
            commands::save_emulator_config,
            commands::validate_emulator_config,
            commands::delete_emulator_config,
            commands::download_asset,
            commands::import_asset_file,
            commands::import_game_file,
            commands::download_game,
            commands::start_game_download,
            commands::trust_executable,
            commands::get_download_root,
            commands::set_download_root,
            commands::get_library_root,
            commands::set_library_root,
            commands::remove_game,
            commands::remove_download,
            commands::redownload_asset,
            commands::open_game_folder,
            commands::open_download_folder,
            commands::open_emulator_folder,
            commands::open_logs_folder,
            commands::run_health_check,
            commands::get_diagnostics_paths,
            commands::get_diagnostics_bundle,
            launcher::launch_game,
            manifest::fetch_manifest,
            orchestrator::install_game,
            orchestrator::install_game_from_manifest,
            orchestrator::add_manifest_source,
            orchestrator::install_emulator,
            orchestrator::get_emulator_status,
            orchestrator::get_emulator_install_status,
            torrent::start_magnet_download,
            torrent::get_torrent_status,
            torrent::get_game_download,
            torrent::list_torrent_downloads,
            torrent::pause_download,
            torrent::resume_download,
            torrent::cancel_download,
            app_update::check_app_update,
            app_update::install_app_update
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Fusion Launcher");
}

pub fn run_package_smoke() -> Result<(), String> {
    let data_dir = std::env::var_os("FUSION_LAUNCHER_PACKAGE_SMOKE_DATA_DIR")
        .or_else(|| std::env::var_os("RETROHYDRA_PACKAGE_SMOKE_DATA_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("fusion-launcher-package-smoke"));
    commands::run_package_smoke(&data_dir)
}

fn prepare_database_path(data_dir: &Path) -> Result<PathBuf, String> {
    let current = data_dir.join("fusion-launcher.db");
    if current.exists() {
        return Ok(current);
    }

    for legacy in legacy_database_candidates(data_dir) {
        if legacy.exists() {
            std::fs::copy(&legacy, &current).map_err(|error| {
                format!(
                    "Failed to migrate legacy database from {} to {}: {error}",
                    legacy.display(),
                    current.display()
                )
            })?;
            return Ok(current);
        }
    }

    Ok(current)
}

fn legacy_database_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![data_dir.join("retrohydra.db")];
    if let Some(parent) = data_dir.parent() {
        candidates.push(parent.join("app.retrohydra.launcher").join("retrohydra.db"));
        candidates.push(parent.join("RetroHydra").join("retrohydra.db"));
    }
    candidates
}

fn setup_error(message: String) -> std::io::Error {
    std::io::Error::other(message)
}

#[cfg(test)]
mod tests {
    use super::prepare_database_path;

    #[test]
    fn migrates_legacy_database_in_current_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("retrohydra.db"), b"legacy").unwrap();

        let path = prepare_database_path(dir.path()).unwrap();

        assert_eq!(path, dir.path().join("fusion-launcher.db"));
        assert_eq!(std::fs::read(path).unwrap(), b"legacy");
    }

    #[test]
    fn keeps_existing_fusion_launcher_database() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fusion-launcher.db"), b"current").unwrap();
        std::fs::write(dir.path().join("retrohydra.db"), b"legacy").unwrap();

        let path = prepare_database_path(dir.path()).unwrap();

        assert_eq!(path, dir.path().join("fusion-launcher.db"));
        assert_eq!(std::fs::read(path).unwrap(), b"current");
    }
}
