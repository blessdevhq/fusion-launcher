use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::Manager;

mod app_update;
mod builtin_demo;
mod commands;
mod downloads;
mod emulator_setup;
mod launcher;
mod logging;
mod platform;
mod schema;
mod security;
mod storage;
mod torrent;

use storage::RepositoryStore;
use torrent::TorrentManager;

pub struct AppState {
    pub store: Arc<Mutex<RepositoryStore>>,
    pub data_dir: PathBuf,
    pub torrents: TorrentManager,
    pub running_games: Arc<Mutex<HashMap<String, u32>>>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| setup_error(error.to_string()))?;
            std::fs::create_dir_all(&data_dir).map_err(|error| setup_error(error.to_string()))?;
            logging::initialize(&data_dir);
            let store = Arc::new(Mutex::new(
                RepositoryStore::open(&data_dir.join("retrohydra.db")).map_err(setup_error)?,
            ));
            let torrents = tauri::async_runtime::block_on(TorrentManager::new(
                data_dir.join("Torrents"),
                data_dir.join("torrent-session"),
                data_dir.clone(),
                Arc::clone(&store),
                app.handle().clone(),
            ))
            .map_err(setup_error)?;

            app.manage(AppState {
                store,
                data_dir,
                torrents,
                running_games: Arc::new(Mutex::new(HashMap::new())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::preview_repository,
            commands::preview_builtin_demo_repository,
            commands::connect_repository,
            commands::connect_builtin_demo_repository,
            commands::refresh_repository,
            commands::get_onboarding_state,
            commands::list_repositories,
            commands::disconnect_repository,
            commands::get_catalog,
            commands::get_game,
            commands::check_requirements,
            commands::get_library_statuses,
            commands::list_emulator_configs,
            commands::save_emulator_config,
            commands::validate_emulator_config,
            commands::delete_emulator_config,
            commands::download_asset,
            commands::download_game,
            commands::start_game_download,
            commands::trust_executable,
            commands::get_download_root,
            commands::set_download_root,
            commands::remove_game,
            commands::redownload_asset,
            commands::open_game_folder,
            commands::open_emulator_folder,
            commands::open_logs_folder,
            commands::run_health_check,
            commands::get_diagnostics_bundle,
            launcher::launch_game,
            torrent::start_magnet_download,
            torrent::get_torrent_status,
            torrent::get_game_download,
            torrent::list_torrent_downloads,
            torrent::pause_download,
            torrent::resume_download,
            torrent::cancel_download,
            emulator_setup::get_recommended_emulators,
            emulator_setup::install_recommended_emulator,
            app_update::check_app_update,
            app_update::install_app_update
        ])
        .run(tauri::generate_context!())
        .expect("failed to run RetroHydra");
}

fn setup_error(message: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, message)
}
