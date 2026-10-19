//! Thin IPC adapter. The core owns operations, filesystem leases and process groups.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, State};

use crate::fusion_source::adapters::torrent::LibtorrentSidecarAdapter;
use crate::fusion_source::adapters::{PixeldrainUriResolver, SecureHttpsTransport};
use crate::fusion_source::canonical::QualifiedGameId;
use crate::fusion_source::dto::*;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::service::{
    FusionHostAdapters, FusionService, FusionStorage, ProgressSink, ServiceEnvironment,
};
use crate::storage::fusion_source::SqliteFusionStorage;
use crate::AppState;

struct TauriProgress {
    app: AppHandle,
    // Owned service tasks retain the session lock even after their IPC caller
    // and bridge are dropped, until process/writer cleanup has completed.
    _session: Arc<FusionSession>,
}
impl ProgressSink for TauriProgress {
    fn emit(&self, event: &OperationProgress) {
        let _ = self.app.emit("fusion-source:operation-progress", event);
    }
}

/// Instances are pinned to their library root and kept alive for cancellation.
/// Installed games continue to use their recorded root after a settings change.
#[derive(Clone)]
pub struct FusionBridge(Arc<BridgeInner>);
struct BridgeInner {
    app: AppHandle,
    _session: Arc<FusionSession>,
    storage: Arc<SqliteFusionStorage>,
    progress: Arc<dyn ProgressSink>,
    environment: ServiceEnvironment,
    services: Mutex<BTreeMap<PathBuf, FusionService>>,
    service_creation: tokio::sync::Mutex<()>,
    host_tools: tokio::sync::OnceCell<crate::fusion_source::transforms::PatchToolRegistry>,
}

/// The operating system releases this lock on process exit. A second launcher
/// cannot classify the first launcher's live operations as interrupted.
struct FusionSession {
    _file: File,
    _directory: crate::fusion_source::fs_safety::DirectoryGuard,
}

impl FusionSession {
    fn acquire(data_dir: &std::path::Path) -> Result<Self, FusionError> {
        use crate::fusion_source::fs_safety;
        let directory = fs_safety::ensure_directory(data_dir)?;
        let path = data_dir.join("fusion-source-session.lock");
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            fs_safety::check_metadata(&metadata, &path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x00200000);
        }
        let file = options
            .open(&path)
            .map_err(|error| fs_safety::io_error("Open Fusion session lock", &path, error))?;
        fs_safety::check_metadata(
            &file.metadata().map_err(|error| {
                fs_safety::io_error("Inspect Fusion session lock", &path, error)
            })?,
            &path,
        )?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| FusionError::new(ErrorCode::OperationInProgress, "Another Fusion Launcher instance is using this library database. Close it before opening another instance."))?;
        Ok(Self {
            _file: file,
            _directory: directory,
        })
    }
}

impl FusionBridge {
    pub fn new(app: &AppHandle, state: &AppState) -> Result<Self, FusionError> {
        let session = Arc::new(FusionSession::acquire(&state.data_dir)?);
        let storage = Arc::new(SqliteFusionStorage::new(state.store.clone()));
        // Startup-only recovery: opening another root must never interrupt a live task.
        storage.interrupt_operations()?;
        let mut environment = ServiceEnvironment {
            appdata: std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| state.data_dir.clone()),
            local_appdata: std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| state.local_data_dir.clone()),
            display_width: 1920,
            display_height: 1080,
            refresh_rate: 60,
        };
        if let Ok(Some(monitor)) = app.primary_monitor() {
            environment.display_width = monitor.size().width;
            environment.display_height = monitor.size().height;
        }
        let bridge = Self(Arc::new(BridgeInner {
            app: app.clone(),
            _session: session.clone(),
            storage,
            progress: Arc::new(TauriProgress {
                app: app.clone(),
                _session: session,
            }),
            environment,
            services: Mutex::new(BTreeMap::new()),
            service_creation: tokio::sync::Mutex::new(()),
            host_tools: tokio::sync::OnceCell::new(),
        }));
        let initialize = bridge.clone();
        let data_dir = state.data_dir.clone();
        tauri::async_runtime::spawn(async move {
            initialize.host_tools(&data_dir).await;
        });
        Ok(bridge)
    }

    async fn host_tools(
        &self,
        data_dir: &std::path::Path,
    ) -> crate::fusion_source::transforms::PatchToolRegistry {
        self.0
            .host_tools
            .get_or_init(|| async {
                let root = data_dir.to_owned();
                match blocking(move || {
                    crate::fusion_source::host_tools::load_host_patch_tools(&root)
                })
                .await
                {
                    Ok(tools) => {
                        for warning in tools.warnings {
                            crate::logging::log_event(
                                data_dir,
                                "fusion_host_tool_unavailable",
                                &[("message", warning.as_str())],
                            );
                        }
                        tools.registry
                    }
                    Err(error) => {
                        crate::logging::log_event(
                            data_dir,
                            "fusion_host_config_invalid",
                            &[("message", error.message.as_str())],
                        );
                        Default::default()
                    }
                }
            })
            .await
            .clone()
    }

    async fn service(
        &self,
        state: &AppState,
        game: Option<&QualifiedGameId>,
    ) -> Result<FusionService, FusionError> {
        if let Some(game) = game {
            if let Some(service) = self
                .0
                .services
                .lock()
                .map_err(|_| bridge_error())?
                .values()
                .find(|service| service.has_active_game(game))
            {
                return Ok(service.clone());
            }
        }
        let recorded_root = game
            .map(|game| self.0.storage.get_installation(game))
            .transpose()?
            .flatten()
            .filter(|record| record.active_snapshot_id.is_some() || record.status == "staging")
            .and_then(|record| record.library_root)
            .map(PathBuf::from);
        let root = recorded_root.unwrap_or_else(|| super::library_root_for_app_state(state));
        let _creation = self.0.service_creation.lock().await;
        if let Some(service) = self
            .0
            .services
            .lock()
            .map_err(|_| bridge_error())?
            .get(&root)
        {
            return Ok(service.clone());
        }
        let mut adapters = FusionHostAdapters {
            cloud: vec![Arc::new(PixeldrainUriResolver)],
            patch_tools: self.host_tools(&state.data_dir).await,
            ..FusionHostAdapters::default()
        };
        #[cfg(windows)]
        {
            adapters.storefront = Some(Arc::new(
                crate::fusion_source::adapters::storefront::WindowsStorefrontInventory::new(),
            ));
        }
        match LibtorrentSidecarAdapter::probe(self.0.app.clone(), root.clone()).await {
            Ok(torrent) => adapters.torrent = Some(Arc::new(torrent)),
            Err(error) => crate::logging::log_event(
                &state.data_dir,
                "fusion_torrent_bridge_unavailable",
                &[("message", error.message.as_str())],
            ),
        }
        let library = root.clone();
        let storage = self.0.storage.clone();
        let progress = self.0.progress.clone();
        let environment = self.0.environment.clone();
        let service = blocking(move || {
            FusionService::open(
                &library,
                storage,
                Arc::new(SecureHttpsTransport),
                progress,
                environment,
            )?
            .with_host_adapters(adapters)
        })
        .await?;
        self.0
            .services
            .lock()
            .map_err(|_| bridge_error())?
            .insert(root, service.clone());
        Ok(service)
    }

    pub fn has_active_operations(&self) -> bool {
        self.0
            .services
            .lock()
            .map(|services| services.values().any(FusionService::has_active_operations))
            .unwrap_or(true)
    }

    fn cancel_game(&self, game: &QualifiedGameId) -> Result<(), FusionError> {
        // Follow the live operation even if a settings change raced the first
        // command before its root was recorded in SQLite.
        let services: Vec<_> = self
            .0
            .services
            .lock()
            .map_err(|_| bridge_error())?
            .values()
            .cloned()
            .collect();
        for service in services {
            service.game_cancel(game)?;
        }
        Ok(())
    }
}

fn bridge_error() -> FusionError {
    FusionError::new(ErrorCode::Io, "The Fusion IPC worker is unavailable.")
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, FusionError> + Send + 'static,
) -> Result<T, FusionError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| bridge_error())?
}

#[tauri::command]
pub async fn fusion_source_preview(
    url: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<SourcePreview, FusionError> {
    bridge
        .service(&state, None)
        .await?
        .source_preview(&url)
        .await
}
#[tauri::command]
pub async fn fusion_source_add(
    url: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<SourceSummary, FusionError> {
    bridge.service(&state, None).await?.source_add(&url).await
}
#[tauri::command]
pub async fn fusion_source_list(
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<Vec<SourceSummary>, FusionError> {
    let service = bridge.service(&state, None).await?;
    blocking(move || service.source_list()).await
}
#[tauri::command]
pub async fn fusion_source_refresh(
    source_id: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<SourceSummary, FusionError> {
    bridge
        .service(&state, None)
        .await?
        .source_refresh(&source_id)
        .await
}
#[tauri::command]
pub async fn fusion_source_remove(
    source_id: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<(), FusionError> {
    let service = bridge.service(&state, None).await?;
    blocking(move || service.source_remove(&source_id)).await
}
#[tauri::command]
pub async fn fusion_source_list_catalog(
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<Vec<FusionCatalogEntry>, FusionError> {
    let service = bridge.service(&state, None).await?;
    blocking(move || service.source_list_catalog()).await
}
#[tauri::command]
pub async fn fusion_game_get_state(
    game: QualifiedGameId,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<FusionGameState, FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_get_state(&game)
        .await
}
#[tauri::command]
pub async fn fusion_game_set_options(
    game: QualifiedGameId,
    options: GameOptions,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<FusionGameState, FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_set_options(&game, options)
        .await
}
#[tauri::command]
pub async fn fusion_game_install(
    game: QualifiedGameId,
    request: InstallRequest,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<OperationProgress, FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_install(&game, request)
        .await
}
#[tauri::command]
pub async fn fusion_game_update(
    game: QualifiedGameId,
    request: InstallRequest,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<OperationProgress, FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_update(&game, request)
        .await
}
#[tauri::command]
pub async fn fusion_game_cancel(
    game: QualifiedGameId,
    bridge: State<'_, FusionBridge>,
) -> Result<(), FusionError> {
    bridge.cancel_game(&game)
}
#[tauri::command]
pub async fn fusion_game_rollback(
    game: QualifiedGameId,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<OperationProgress, FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_rollback(&game)
        .await
}
#[tauri::command]
pub async fn fusion_game_launch(
    game: QualifiedGameId,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<OperationProgress, FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_launch(&game)
        .await
}
#[tauri::command]
pub async fn fusion_game_remove(
    game: QualifiedGameId,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<OperationProgress, FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_remove(&game)
        .await
}
#[tauri::command]
pub async fn fusion_game_trust_executable(
    game: QualifiedGameId,
    path: String,
    blake3: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<(), FusionError> {
    bridge
        .service(&state, Some(&game))
        .await?
        .game_trust_executable(&game, &path, &blake3)
        .await
}
#[tauri::command]
pub async fn fusion_crash_preview(
    report_id: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<CrashPreview, FusionError> {
    let service = bridge.service(&state, None).await?;
    blocking(move || service.crash_preview(&report_id)).await
}
#[tauri::command]
pub async fn fusion_crash_send(
    report_id: String,
    expected_endpoint: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<(), FusionError> {
    let service = bridge.service(&state, None).await?;
    let target = service.crash_send_target(&report_id)?;
    if target.endpoint() != expected_endpoint {
        return Err(FusionError::new(
            ErrorCode::SnapshotConflict,
            "The report destination changed. Review the report again before sending it.",
        ));
    }
    let url = crate::fusion_source::adapters::https_url(target.endpoint())?;
    crate::net::post_external_json(&url, target.body(), Duration::from_secs(30))
        .await
        .map_err(|message| FusionError::new(ErrorCode::Network, message))?;
    blocking(move || service.crash_mark_sent(&report_id)).await
}
#[tauri::command]
pub async fn fusion_crash_discard(
    report_id: String,
    state: State<'_, AppState>,
    bridge: State<'_, FusionBridge>,
) -> Result<(), FusionError> {
    let service = bridge.service(&state, None).await?;
    blocking(move || service.crash_discard(&report_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_recovery_requires_exclusive_database_owner() {
        let directory = tempfile::tempdir().unwrap();
        let session = FusionSession::acquire(directory.path()).unwrap();
        assert_eq!(
            FusionSession::acquire(directory.path()).err().unwrap().code,
            ErrorCode::OperationInProgress
        );
        drop(session);
        assert!(FusionSession::acquire(directory.path()).is_ok());
    }
}
