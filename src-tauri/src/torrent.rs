use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StoreMutex};
use std::time::{Duration, Instant};

use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, PeerConnectionOptions,
    Session, SessionOptions, SessionPersistenceConfig,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::schema::TorrentDownloadRecord;
use crate::storage::RepositoryStore;
use crate::AppState;

const SESSION_UPLOAD_LIMIT_BYTES_PER_SEC: u32 = 256 * 1024;
const TORRENT_UPLOAD_LIMIT_BYTES_PER_SEC: u32 = 256 * 1024;
const MAX_ACTIVE_DOWNLOADS: usize = 1;
const PEER_CONNECT_TIMEOUT_SECS: u64 = 8;
const PEER_READ_WRITE_TIMEOUT_SECS: u64 = 30;
const PEER_KEEP_ALIVE_INTERVAL_SECS: u64 = 120;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const DB_FLUSH_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE_DOWNLOAD_LIMIT_ERROR: &str =
    "Another torrent download is already active. Please wait for it to finish before starting a new one.";

pub struct TorrentManager {
    session: Arc<Session>,
    records: Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: Arc<StoreMutex<RepositoryStore>>,
    app: AppHandle,
    data_dir: PathBuf,
}

#[derive(Clone)]
struct TorrentRecord {
    game_id: String,
    save_dir: String,
    torrent_id: Option<usize>,
    handle: Option<Arc<ManagedTorrent>>,
    state: TorrentRecordState,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TorrentRecordState {
    Resolving,
    Downloading,
    Paused,
    Completed,
    Cancelling,
    Error,
}

impl TorrentRecordState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Resolving => "resolving",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Cancelling => "cancelling",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentStartReport {
    pub game_id: String,
    pub state: String,
    pub save_dir: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentStatus {
    pub game_id: String,
    pub state: String,
    pub progress: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed_bytes_per_sec: u64,
    pub upload_speed_bytes_per_sec: u64,
    pub peers_count: usize,
    pub finished: bool,
    pub save_dir: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgressEvent {
    pub game_id: String,
    pub status: String,
    pub progress: f64,
    pub progress_percent: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed_bytes_per_sec: u64,
    pub upload_speed_bytes_per_sec: u64,
    pub peers_count: usize,
    pub finished: bool,
    pub save_dir: String,
    pub error: Option<String>,
}

impl TorrentManager {
    pub async fn new(
        default_output_folder: PathBuf,
        persistence_folder: PathBuf,
        data_dir: PathBuf,
        store: Arc<StoreMutex<RepositoryStore>>,
        app: AppHandle,
    ) -> Result<Self, String> {
        tokio::fs::create_dir_all(&default_output_folder)
            .await
            .map_err(|error| format!("Failed to create torrent output directory: {error}"))?;
        tokio::fs::create_dir_all(&persistence_folder)
            .await
            .map_err(|error| format!("Failed to create torrent session directory: {error}"))?;

        let session = Session::new_with_opts(
            default_output_folder,
            conservative_session_options(persistence_folder),
        )
        .await
        .map_err(|error| format!("Failed to initialize torrent session: {error:#}"))?;

        let manager = Self {
            session,
            records: Arc::new(Mutex::new(HashMap::new())),
            store,
            app,
            data_dir,
        };
        manager.restore_startup_downloads().await;
        Ok(manager)
    }

    pub async fn start_magnet_download(
        &self,
        game_id: String,
        magnet_uri: String,
        save_dir: String,
    ) -> Result<TorrentStartReport, String> {
        validate_start_request(&game_id, &magnet_uri, &save_dir)?;

        let game_id = game_id.trim().to_string();
        let magnet_uri = magnet_uri.trim().to_string();
        let save_dir = save_dir.trim().to_string();
        let output_folder = PathBuf::from(&save_dir);

        {
            let records = self.records.lock().await;
            ensure_can_start_download(&records, &game_id)?;
        }

        tokio::fs::create_dir_all(&output_folder)
            .await
            .map_err(|error| format!("Failed to create save directory: {error}"))?;

        {
            let store = lock_store(&self.store)?;
            store.upsert_torrent_download_start(&game_id, &magnet_uri, &save_dir)?;
        }

        {
            let mut records = self.records.lock().await;
            ensure_can_start_download(&records, &game_id)?;

            records.insert(
                game_id.clone(),
                TorrentRecord {
                    game_id: game_id.clone(),
                    save_dir: save_dir.clone(),
                    torrent_id: None,
                    handle: None,
                    state: TorrentRecordState::Resolving,
                    error: None,
                },
            );
        }

        self.spawn_add_torrent_task(game_id.clone(), magnet_uri, save_dir.clone());

        Ok(TorrentStartReport {
            game_id,
            state: TorrentRecordState::Resolving.as_str().to_string(),
            save_dir: output_folder.to_string_lossy().to_string(),
        })
    }

    pub async fn get_torrent_status(&self, game_id: String) -> Result<TorrentStatus, String> {
        if let Some(status) = status_from_records(&self.records, game_id.clone()).await? {
            return Ok(status);
        }

        let store = lock_store(&self.store)?;
        store
            .get_torrent_download(&game_id)?
            .map(|record| status_from_persisted_record(&record))
            .ok_or_else(|| format!("Unknown torrent: {game_id}"))
    }

    pub fn get_game_download(
        &self,
        game_id: &str,
    ) -> Result<Option<TorrentDownloadRecord>, String> {
        lock_store(&self.store)?.get_torrent_download(game_id)
    }

    pub fn list_downloads(&self) -> Result<Vec<TorrentDownloadRecord>, String> {
        lock_store(&self.store)?.list_torrent_downloads()
    }

    pub async fn pause_download(&self, game_id: String) -> Result<TorrentDownloadRecord, String> {
        let handle = {
            let records = self.records.lock().await;
            records
                .get(&game_id)
                .and_then(|record| record.handle.clone())
        };

        if let Some(handle) = handle {
            self.session
                .pause(&handle)
                .await
                .map_err(|error| format!("Failed to pause torrent: {error:#}"))?;
        }

        {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(&game_id) {
                record.state = TorrentRecordState::Paused;
                record.error = None;
            }
        }

        let record = lock_store(&self.store)?.set_torrent_status(&game_id, "paused", None)?;
        emit_download_record(&self.app, &record);
        Ok(record)
    }

    pub async fn resume_download(&self, game_id: String) -> Result<TorrentDownloadRecord, String> {
        let handle = {
            let records = self.records.lock().await;
            ensure_can_start_download_excluding(&records, &game_id)?;
            records
                .get(&game_id)
                .and_then(|record| record.handle.clone())
        };

        if let Some(handle) = handle {
            self.session
                .unpause(&handle)
                .await
                .map_err(|error| format!("Failed to resume torrent: {error:#}"))?;

            {
                let mut records = self.records.lock().await;
                if let Some(record) = records.get_mut(&game_id) {
                    record.state = TorrentRecordState::Downloading;
                    record.error = None;
                }
            }

            let record =
                lock_store(&self.store)?.set_torrent_status(&game_id, "downloading", None)?;
            emit_download_record(&self.app, &record);
            self.spawn_polling_for_game(game_id, handle);
            return Ok(record);
        }

        let record = {
            let store = lock_store(&self.store)?;
            store
                .get_torrent_download(&game_id)?
                .ok_or_else(|| format!("Unknown torrent download: {game_id}"))?;
            store.set_torrent_status(&game_id, "resolving", None)?
        };

        {
            let mut records = self.records.lock().await;
            ensure_can_start_download_excluding(&records, &game_id)?;
            records.insert(
                record.game_id.clone(),
                TorrentRecord::from_persisted(&record, TorrentRecordState::Resolving, None),
            );
        }

        tokio::fs::create_dir_all(&record.save_dir)
            .await
            .map_err(|error| format!("Failed to create save directory: {error}"))?;
        self.spawn_add_torrent_task(
            record.game_id.clone(),
            record.magnet_uri.clone(),
            record.save_dir.clone(),
        );

        emit_download_record(&self.app, &record);
        Ok(record)
    }

    pub async fn cancel_download(&self, game_id: String) -> Result<TorrentDownloadRecord, String> {
        let record = lock_store(&self.store)?
            .get_torrent_download(&game_id)?
            .ok_or_else(|| format!("Unknown torrent download: {game_id}"))?;

        if record.status == "completed" {
            return Err("Completed downloads cannot be cancelled.".to_string());
        }

        let cancelling =
            lock_store(&self.store)?.set_torrent_status(&game_id, "cancelling", None)?;
        emit_download_record(&self.app, &cancelling);

        let (runtime_handle, runtime_id) = {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(&game_id) {
                record.state = TorrentRecordState::Cancelling;
                (record.handle.clone(), record.torrent_id)
            } else {
                (None, None)
            }
        };

        let torrent_id = runtime_id.or_else(|| record.torrent_id.and_then(i64_to_usize));
        let mut delete_error = None;
        if let Some(id) = torrent_id {
            if let Err(error) = self.session.delete(TorrentIdOrHash::Id(id), true).await {
                delete_error = Some(format!("Failed to delete torrent session files: {error:#}"));
            }
        } else if let Some(handle) = runtime_handle {
            if let Err(error) = self
                .session
                .delete(TorrentIdOrHash::Id(handle.id()), true)
                .await
            {
                delete_error = Some(format!("Failed to delete torrent session files: {error:#}"));
            }
        }

        if let Err(cleanup_error) = remove_save_dir_if_safe(&self.data_dir, &record.save_dir).await
        {
            let error = match delete_error {
                Some(delete_error) => format!("{delete_error}; {cleanup_error}"),
                None => cleanup_error,
            };
            let record =
                lock_store(&self.store)?.set_torrent_status(&game_id, "error", Some(&error))?;
            set_runtime_error(&self.records, &game_id, error).await;
            emit_download_record(&self.app, &record);
            return Ok(record);
        }

        {
            let mut records = self.records.lock().await;
            records.remove(&game_id);
        }

        let cancelled = lock_store(&self.store)?.set_torrent_status(&game_id, "cancelled", None)?;
        emit_download_record(&self.app, &cancelled);
        Ok(cancelled)
    }

    async fn restore_startup_downloads(&self) {
        let startup_records = match lock_store(&self.store)
            .and_then(|store| store.list_startup_torrent_downloads())
        {
            Ok(records) => records,
            Err(_) => return,
        };

        for record in startup_records {
            match record.status.as_str() {
                "cancelling" => {
                    let _ = self.finish_startup_cancellation(record).await;
                }
                "paused" => {
                    self.restore_paused_record(record).await;
                }
                "resolving" | "downloading" | "interrupted" => {
                    let interrupted = match lock_store(&self.store)
                        .and_then(|store| store.mark_torrent_interrupted(&record.game_id))
                    {
                        Ok(record) => record,
                        Err(_) => record,
                    };
                    let _ = self.resume_persisted_record(interrupted).await;
                }
                _ => {}
            }
        }
    }

    async fn finish_startup_cancellation(
        &self,
        record: TorrentDownloadRecord,
    ) -> Result<(), String> {
        if let Some(id) = record.torrent_id.and_then(i64_to_usize) {
            let _ = self.session.delete(TorrentIdOrHash::Id(id), true).await;
        }
        remove_save_dir_if_safe(&self.data_dir, &record.save_dir).await?;
        let cancelled =
            lock_store(&self.store)?.set_torrent_status(&record.game_id, "cancelled", None)?;
        emit_download_record(&self.app, &cancelled);
        Ok(())
    }

    async fn restore_paused_record(&self, record: TorrentDownloadRecord) {
        let handle = self.handle_for_persisted_record(&record);
        if let Some(handle) = handle.as_ref() {
            let _ = self.session.pause(handle).await;
        }

        if handle.is_some() {
            let mut records = self.records.lock().await;
            records.insert(
                record.game_id.clone(),
                TorrentRecord::from_persisted(&record, TorrentRecordState::Paused, handle),
            );
        }
    }

    async fn resume_persisted_record(&self, record: TorrentDownloadRecord) -> Result<(), String> {
        {
            let records = self.records.lock().await;
            ensure_can_start_download_excluding(&records, &record.game_id)?;
        }

        tokio::fs::create_dir_all(&record.save_dir)
            .await
            .map_err(|error| format!("Failed to create save directory: {error}"))?;

        {
            let mut records = self.records.lock().await;
            ensure_can_start_download_excluding(&records, &record.game_id)?;
            records.insert(
                record.game_id.clone(),
                TorrentRecord::from_persisted(&record, TorrentRecordState::Resolving, None),
            );
        }

        self.spawn_add_torrent_task(
            record.game_id.clone(),
            record.magnet_uri.clone(),
            record.save_dir.clone(),
        );
        Ok(())
    }

    fn handle_for_persisted_record(
        &self,
        record: &TorrentDownloadRecord,
    ) -> Option<Arc<ManagedTorrent>> {
        record
            .torrent_id
            .and_then(i64_to_usize)
            .and_then(|id| self.session.get(TorrentIdOrHash::Id(id)))
    }

    fn spawn_add_torrent_task(&self, game_id: String, magnet_uri: String, save_dir: String) {
        let session = Arc::clone(&self.session);
        let records = Arc::clone(&self.records);
        let store = Arc::clone(&self.store);
        let app = self.app.clone();

        tauri::async_runtime::spawn(async move {
            let add_result = session
                .add_torrent(
                    AddTorrent::from_url(magnet_uri),
                    Some(conservative_add_torrent_options(save_dir)),
                )
                .await;

            match add_result {
                Ok(AddTorrentResponse::Added(id, handle))
                | Ok(AddTorrentResponse::AlreadyManaged(id, handle)) => {
                    let desired_state = {
                        let mut records = records.lock().await;
                        records.get_mut(&game_id).map(|record| {
                            record.torrent_id = Some(id);
                            record.handle = Some(Arc::clone(&handle));
                            record.error = None;
                            match record.state {
                                TorrentRecordState::Paused => TorrentRecordState::Paused,
                                TorrentRecordState::Cancelling => record.state,
                                _ => {
                                    record.state = TorrentRecordState::Downloading;
                                    TorrentRecordState::Downloading
                                }
                            }
                        })
                    };

                    match desired_state {
                        Some(TorrentRecordState::Paused) => {
                            let _ = session.pause(&handle).await;
                            if let Ok(record) = lock_store(&store).and_then(|store| {
                                store.update_torrent_handle(&game_id, id as i64, "paused")
                            }) {
                                emit_download_record(&app, &record);
                            }
                        }
                        Some(TorrentRecordState::Cancelling) => {
                            let _ = session.delete(TorrentIdOrHash::Id(id), true).await;
                            if let Ok(record) = lock_store(&store).and_then(|store| {
                                store.set_torrent_status(&game_id, "cancelled", None)
                            }) {
                                emit_download_record(&app, &record);
                            }
                        }
                        Some(_) => {
                            let _ = session.unpause(&handle).await;
                            if let Ok(record) = lock_store(&store).and_then(|store| {
                                store.update_torrent_handle(&game_id, id as i64, "downloading")
                            }) {
                                emit_download_record(&app, &record);
                            }
                            spawn_polling_task(session, records, store, app, game_id, handle);
                        }
                        None => {
                            let _ = session.delete(TorrentIdOrHash::Id(id), true).await;
                        }
                    }
                }
                Ok(AddTorrentResponse::ListOnly(_)) => {
                    set_torrent_error(
                        &records,
                        &store,
                        &app,
                        &game_id,
                        "Torrent was added in list-only mode unexpectedly.".to_string(),
                    )
                    .await;
                }
                Err(error) => {
                    set_torrent_error(
                        &records,
                        &store,
                        &app,
                        &game_id,
                        format!("Failed to start magnet download: {error:#}"),
                    )
                    .await;
                }
            }
        });
    }

    fn spawn_polling_for_game(&self, game_id: String, handle: Arc<ManagedTorrent>) {
        spawn_polling_task(
            Arc::clone(&self.session),
            Arc::clone(&self.records),
            Arc::clone(&self.store),
            self.app.clone(),
            game_id,
            handle,
        );
    }
}

impl TorrentRecord {
    fn from_persisted(
        record: &TorrentDownloadRecord,
        state: TorrentRecordState,
        handle: Option<Arc<ManagedTorrent>>,
    ) -> Self {
        Self {
            game_id: record.game_id.clone(),
            save_dir: record.save_dir.clone(),
            torrent_id: record.torrent_id.and_then(i64_to_usize),
            handle,
            state,
            error: record.error_message.clone(),
        }
    }
}

#[tauri::command]
pub async fn start_magnet_download(
    game_id: String,
    magnet_uri: String,
    save_dir: String,
    state: State<'_, AppState>,
) -> Result<TorrentStartReport, String> {
    state
        .torrents
        .start_magnet_download(game_id, magnet_uri, save_dir)
        .await
}

#[tauri::command]
pub async fn get_torrent_status(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<TorrentStatus, String> {
    state.torrents.get_torrent_status(game_id).await
}

#[tauri::command]
pub fn get_game_download(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Option<TorrentDownloadRecord>, String> {
    state.torrents.get_game_download(&game_id)
}

#[tauri::command]
pub fn list_torrent_downloads(
    state: State<'_, AppState>,
) -> Result<Vec<TorrentDownloadRecord>, String> {
    state.torrents.list_downloads()
}

#[tauri::command]
pub async fn pause_download(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<TorrentDownloadRecord, String> {
    state.torrents.pause_download(game_id).await
}

#[tauri::command]
pub async fn resume_download(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<TorrentDownloadRecord, String> {
    state.torrents.resume_download(game_id).await
}

#[tauri::command]
pub async fn cancel_download(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<TorrentDownloadRecord, String> {
    state.torrents.cancel_download(game_id).await
}

fn validate_start_request(game_id: &str, magnet_uri: &str, save_dir: &str) -> Result<(), String> {
    if game_id.trim().is_empty() {
        return Err("Game id cannot be empty.".to_string());
    }
    if !magnet_uri.trim_start().starts_with("magnet:?") {
        return Err("Invalid magnet URI.".to_string());
    }
    if save_dir.trim().is_empty() {
        return Err("Save directory cannot be empty.".to_string());
    }

    Ok(())
}

fn conservative_session_options(persistence_folder: PathBuf) -> SessionOptions {
    SessionOptions {
        peer_opts: Some(conservative_peer_options()),
        listen_port_range: None,
        enable_upnp_port_forwarding: false,
        concurrent_init_limit: Some(MAX_ACTIVE_DOWNLOADS),
        ratelimits: upload_ratelimits(SESSION_UPLOAD_LIMIT_BYTES_PER_SEC),
        fastresume: true,
        persistence: Some(SessionPersistenceConfig::Json {
            folder: Some(persistence_folder),
        }),
        ..Default::default()
    }
}

fn conservative_add_torrent_options(output_folder: String) -> AddTorrentOptions {
    AddTorrentOptions {
        output_folder: Some(output_folder),
        overwrite: true,
        peer_opts: Some(conservative_peer_options()),
        ratelimits: upload_ratelimits(TORRENT_UPLOAD_LIMIT_BYTES_PER_SEC),
        ..Default::default()
    }
}

fn conservative_peer_options() -> PeerConnectionOptions {
    PeerConnectionOptions {
        connect_timeout: Some(Duration::from_secs(PEER_CONNECT_TIMEOUT_SECS)),
        read_write_timeout: Some(Duration::from_secs(PEER_READ_WRITE_TIMEOUT_SECS)),
        keep_alive_interval: Some(Duration::from_secs(PEER_KEEP_ALIVE_INTERVAL_SECS)),
    }
}

fn upload_ratelimits(upload_bytes_per_sec: u32) -> LimitsConfig {
    LimitsConfig {
        upload_bps: Some(
            NonZeroU32::new(upload_bytes_per_sec).expect("upload limit must be non-zero"),
        ),
        download_bps: None,
    }
}

fn ensure_can_start_download(
    records: &HashMap<String, TorrentRecord>,
    game_id: &str,
) -> Result<(), String> {
    if let Some(record) = records.get(game_id) {
        if !matches!(
            record.state,
            TorrentRecordState::Completed | TorrentRecordState::Error
        ) {
            return Err(format!("Torrent already tracked for game: {game_id}"));
        }
    }

    if active_download_count(records) >= MAX_ACTIVE_DOWNLOADS {
        return Err(ACTIVE_DOWNLOAD_LIMIT_ERROR.to_string());
    }

    Ok(())
}

fn ensure_can_start_download_excluding(
    records: &HashMap<String, TorrentRecord>,
    game_id: &str,
) -> Result<(), String> {
    if active_download_count_excluding(records, game_id) >= MAX_ACTIVE_DOWNLOADS {
        return Err(ACTIVE_DOWNLOAD_LIMIT_ERROR.to_string());
    }

    Ok(())
}

fn active_download_count(records: &HashMap<String, TorrentRecord>) -> usize {
    records
        .values()
        .filter(|record| record_blocks_new_download(record))
        .count()
}

fn active_download_count_excluding(
    records: &HashMap<String, TorrentRecord>,
    game_id: &str,
) -> usize {
    records
        .values()
        .filter(|record| record.game_id != game_id)
        .filter(|record| record_blocks_new_download(record))
        .count()
}

fn record_blocks_new_download(record: &TorrentRecord) -> bool {
    matches!(
        record.state,
        TorrentRecordState::Resolving
            | TorrentRecordState::Downloading
            | TorrentRecordState::Cancelling
    )
}

async fn status_from_records(
    records: &Arc<Mutex<HashMap<String, TorrentRecord>>>,
    game_id: String,
) -> Result<Option<TorrentStatus>, String> {
    let record = {
        let records = records.lock().await;
        records.get(&game_id).cloned()
    };

    let Some(record) = record else {
        return Ok(None);
    };

    let Some(handle) = record.handle.clone() else {
        return Ok(Some(pending_status(record)));
    };

    Ok(Some(status_from_handle(&record, &handle)))
}

fn pending_status(record: TorrentRecord) -> TorrentStatus {
    TorrentStatus {
        game_id: record.game_id,
        state: record.state.as_str().to_string(),
        progress: 0.0,
        downloaded_bytes: 0,
        total_bytes: 0,
        download_speed_bytes_per_sec: 0,
        upload_speed_bytes_per_sec: 0,
        peers_count: 0,
        finished: false,
        save_dir: record.save_dir,
        error: record.error,
    }
}

fn status_from_handle(record: &TorrentRecord, handle: &Arc<ManagedTorrent>) -> TorrentStatus {
    let stats = handle.stats();
    let progress = if stats.total_bytes == 0 {
        0.0
    } else {
        stats.progress_bytes as f64 / stats.total_bytes as f64
    };

    let (download_speed_bytes_per_sec, upload_speed_bytes_per_sec, peers_count) =
        if let Some(live) = &stats.live {
            (
                mib_per_sec_to_bytes_per_sec(live.download_speed.mbps),
                mib_per_sec_to_bytes_per_sec(live.upload_speed.mbps),
                live.snapshot.peer_stats.live,
            )
        } else {
            (0, 0, 0)
        };

    let error = stats.error.or_else(|| record.error.clone());
    let state = if error.is_some() {
        TorrentRecordState::Error.as_str()
    } else if stats.finished || progress >= 1.0 {
        TorrentRecordState::Completed.as_str()
    } else {
        record.state.as_str()
    };

    TorrentStatus {
        game_id: record.game_id.clone(),
        state: state.to_string(),
        progress: clamp_progress(progress),
        downloaded_bytes: stats.progress_bytes,
        total_bytes: stats.total_bytes,
        download_speed_bytes_per_sec,
        upload_speed_bytes_per_sec,
        peers_count,
        finished: stats.finished || progress >= 1.0,
        save_dir: record.save_dir.clone(),
        error,
    }
}

fn status_from_persisted_record(record: &TorrentDownloadRecord) -> TorrentStatus {
    TorrentStatus {
        game_id: record.game_id.clone(),
        state: record.status.clone(),
        progress: clamp_progress(record.progress_percent / 100.0),
        downloaded_bytes: record.downloaded_bytes,
        total_bytes: record.total_bytes,
        download_speed_bytes_per_sec: record.download_speed_bytes_per_sec,
        upload_speed_bytes_per_sec: record.upload_speed_bytes_per_sec,
        peers_count: record.peers_count,
        finished: record.status == "completed",
        save_dir: record.save_dir.clone(),
        error: record.error_message.clone(),
    }
}

fn spawn_polling_task(
    session: Arc<Session>,
    records: Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: Arc<StoreMutex<RepositoryStore>>,
    app: AppHandle,
    game_id: String,
    handle: Arc<ManagedTorrent>,
) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        let mut last_flush = Instant::now() - DB_FLUSH_INTERVAL;
        let mut last_status = String::new();

        loop {
            interval.tick().await;

            let record = {
                let records = records.lock().await;
                records.get(&game_id).cloned()
            };

            let Some(record) = record else {
                break;
            };

            if matches!(
                record.state,
                TorrentRecordState::Paused
                    | TorrentRecordState::Cancelling
                    | TorrentRecordState::Completed
                    | TorrentRecordState::Error
            ) {
                break;
            }

            let status = status_from_handle(&record, &handle);
            let event = event_from_status(&status);
            let _ = app.emit("download:progress", event);

            let status_changed = status.state != last_status;
            let should_flush = last_flush.elapsed() >= DB_FLUSH_INTERVAL
                || status_changed
                || status.finished
                || status.error.is_some();

            if should_flush {
                last_status = status.state.clone();
                last_flush = Instant::now();

                let persisted = if status.finished {
                    lock_store(&store).and_then(|store| {
                        store.mark_torrent_completed(
                            &game_id,
                            &status.save_dir,
                            status.downloaded_bytes,
                            status.total_bytes,
                        )
                    })
                } else {
                    lock_store(&store).and_then(|store| {
                        store.update_torrent_progress(
                            &game_id,
                            &status.state,
                            status.progress * 100.0,
                            status.downloaded_bytes,
                            status.total_bytes,
                            status.download_speed_bytes_per_sec,
                            status.upload_speed_bytes_per_sec,
                            status.peers_count,
                        )
                    })
                };

                if let Ok(record) = persisted {
                    emit_download_record(&app, &record);
                }
            }

            if let Some(error) = status.error {
                set_torrent_error(&records, &store, &app, &game_id, error).await;
                break;
            }

            if status.finished {
                {
                    let mut records = records.lock().await;
                    if let Some(record) = records.get_mut(&game_id) {
                        record.state = TorrentRecordState::Completed;
                        record.error = None;
                    }
                }
                let _ = session
                    .delete(TorrentIdOrHash::Id(handle.id()), false)
                    .await;
                break;
            }
        }
    });
}

async fn set_torrent_error(
    records: &Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: &Arc<StoreMutex<RepositoryStore>>,
    app: &AppHandle,
    game_id: &str,
    error: String,
) {
    set_runtime_error(records, game_id, error.clone()).await;
    if let Ok(record) =
        lock_store(store).and_then(|store| store.set_torrent_status(game_id, "error", Some(&error)))
    {
        emit_download_record(app, &record);
    }
}

async fn set_runtime_error(
    records: &Arc<Mutex<HashMap<String, TorrentRecord>>>,
    game_id: &str,
    error: String,
) {
    let mut records = records.lock().await;
    if let Some(record) = records.get_mut(game_id) {
        record.state = TorrentRecordState::Error;
        record.error = Some(error);
    }
}

fn event_from_status(status: &TorrentStatus) -> DownloadProgressEvent {
    DownloadProgressEvent {
        game_id: status.game_id.clone(),
        status: status.state.clone(),
        progress: status.progress,
        progress_percent: status.progress * 100.0,
        downloaded_bytes: status.downloaded_bytes,
        total_bytes: status.total_bytes,
        download_speed_bytes_per_sec: status.download_speed_bytes_per_sec,
        upload_speed_bytes_per_sec: status.upload_speed_bytes_per_sec,
        peers_count: status.peers_count,
        finished: status.finished,
        save_dir: status.save_dir.clone(),
        error: status.error.clone(),
    }
}

fn emit_download_record(app: &AppHandle, record: &TorrentDownloadRecord) {
    let status = status_from_persisted_record(record);
    let _ = app.emit("download:progress", event_from_status(&status));
}

async fn remove_save_dir_if_safe(data_dir: &Path, save_dir: &str) -> Result<(), String> {
    let save_dir = PathBuf::from(save_dir);
    if !save_dir.exists() {
        return Ok(());
    }

    let canonical_data_dir = tokio::fs::canonicalize(data_dir)
        .await
        .map_err(|error| format!("Failed to inspect app data directory: {error}"))?;
    let canonical_save_dir = tokio::fs::canonicalize(&save_dir)
        .await
        .map_err(|error| format!("Failed to inspect save directory: {error}"))?;

    if canonical_save_dir == canonical_data_dir
        || !canonical_save_dir.starts_with(&canonical_data_dir)
    {
        return Err(format!(
            "Refusing to delete download folder outside app data: {}",
            save_dir.display()
        ));
    }

    tokio::fs::remove_dir_all(&canonical_save_dir)
        .await
        .map_err(|error| format!("Failed to delete partial download files: {error}"))
}

fn lock_store(
    store: &Arc<StoreMutex<RepositoryStore>>,
) -> Result<std::sync::MutexGuard<'_, RepositoryStore>, String> {
    store
        .lock()
        .map_err(|_| "Repository store lock is poisoned.".to_string())
}

fn i64_to_usize(value: i64) -> Option<usize> {
    usize::try_from(value).ok()
}

fn clamp_progress(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }

    value.clamp(0.0, 1.0)
}

fn mib_per_sec_to_bytes_per_sec(mibps: f64) -> u64 {
    if !mibps.is_finite() || mibps <= 0.0 {
        return 0;
    }

    (mibps * 1024.0 * 1024.0).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_record(game_id: &str, state: TorrentRecordState) -> TorrentRecord {
        TorrentRecord {
            game_id: game_id.to_string(),
            save_dir: "F:/Downloads/game".to_string(),
            torrent_id: None,
            handle: None,
            state,
            error: None,
        }
    }

    #[test]
    fn blank_game_id_errors() {
        assert_eq!(
            validate_start_request(" ", "magnet:?xt=urn:btih:abc", "F:/Downloads").unwrap_err(),
            "Game id cannot be empty."
        );
    }

    #[test]
    fn non_magnet_uri_errors() {
        assert_eq!(
            validate_start_request("game", "https://example.com/game.torrent", "F:/Downloads")
                .unwrap_err(),
            "Invalid magnet URI."
        );
    }

    #[test]
    fn blank_save_dir_errors() {
        assert_eq!(
            validate_start_request("game", "magnet:?xt=urn:btih:abc", " ").unwrap_err(),
            "Save directory cannot be empty."
        );
    }

    #[test]
    fn session_options_set_conservative_network_limits() {
        let options = conservative_session_options(PathBuf::from("F:/Session"));
        let peer_opts = options.peer_opts.expect("peer options should be set");

        assert_eq!(
            options.ratelimits.upload_bps.map(NonZeroU32::get),
            Some(SESSION_UPLOAD_LIMIT_BYTES_PER_SEC)
        );
        assert_eq!(options.ratelimits.download_bps, None);
        assert_eq!(options.concurrent_init_limit, Some(MAX_ACTIVE_DOWNLOADS));
        assert_eq!(options.listen_port_range, None);
        assert!(!options.enable_upnp_port_forwarding);
        assert!(options.fastresume);
        assert!(options.persistence.is_some());
        assert_eq!(
            peer_opts.connect_timeout,
            Some(Duration::from_secs(PEER_CONNECT_TIMEOUT_SECS))
        );
        assert_eq!(
            peer_opts.read_write_timeout,
            Some(Duration::from_secs(PEER_READ_WRITE_TIMEOUT_SECS))
        );
        assert_eq!(
            peer_opts.keep_alive_interval,
            Some(Duration::from_secs(PEER_KEEP_ALIVE_INTERVAL_SECS))
        );
    }

    #[test]
    fn add_torrent_options_set_conservative_network_limits() {
        let options = conservative_add_torrent_options("F:/Downloads/game".to_string());
        let peer_opts = options.peer_opts.expect("peer options should be set");

        assert_eq!(options.output_folder.as_deref(), Some("F:/Downloads/game"));
        assert!(options.overwrite);
        assert_eq!(
            options.ratelimits.upload_bps.map(NonZeroU32::get),
            Some(TORRENT_UPLOAD_LIMIT_BYTES_PER_SEC)
        );
        assert_eq!(options.ratelimits.download_bps, None);
        assert_eq!(
            peer_opts.connect_timeout,
            Some(Duration::from_secs(PEER_CONNECT_TIMEOUT_SECS))
        );
        assert_eq!(
            peer_opts.read_write_timeout,
            Some(Duration::from_secs(PEER_READ_WRITE_TIMEOUT_SECS))
        );
        assert_eq!(
            peer_opts.keep_alive_interval,
            Some(Duration::from_secs(PEER_KEEP_ALIVE_INTERVAL_SECS))
        );
    }

    #[test]
    fn active_download_guard_blocks_when_resolving_record_exists() {
        let mut records = HashMap::new();
        records.insert(
            "game-1".to_string(),
            test_record("game-1", TorrentRecordState::Resolving),
        );

        assert_eq!(
            ensure_can_start_download(&records, "game-2").unwrap_err(),
            ACTIVE_DOWNLOAD_LIMIT_ERROR
        );
    }

    #[test]
    fn active_download_guard_allows_when_only_error_record_exists() {
        let mut records = HashMap::new();
        let mut record = test_record("game-1", TorrentRecordState::Error);
        record.error = Some("network error".to_string());
        records.insert("game-1".to_string(), record);

        assert!(ensure_can_start_download(&records, "game-2").is_ok());
    }

    #[test]
    fn active_download_guard_allows_when_existing_record_is_paused() {
        let mut records = HashMap::new();
        records.insert(
            "game-1".to_string(),
            test_record("game-1", TorrentRecordState::Paused),
        );

        assert!(ensure_can_start_download(&records, "game-2").is_ok());
    }

    #[test]
    fn duplicate_game_id_error_takes_precedence_for_paused_download() {
        let mut records = HashMap::new();
        records.insert(
            "game-1".to_string(),
            test_record("game-1", TorrentRecordState::Paused),
        );

        assert_eq!(
            ensure_can_start_download(&records, "game-1").unwrap_err(),
            "Torrent already tracked for game: game-1"
        );
    }

    #[test]
    fn unknown_game_status_returns_none() {
        let records = Arc::new(Mutex::new(HashMap::new()));
        let status = tauri::async_runtime::block_on(status_from_records(
            &records,
            "missing-game".to_string(),
        ))
        .unwrap();

        assert!(status.is_none());
    }

    #[test]
    fn resolving_record_reports_zero_progress() {
        let records = Arc::new(Mutex::new(HashMap::new()));
        tauri::async_runtime::block_on(async {
            records.lock().await.insert(
                "game-1".to_string(),
                test_record("game-1", TorrentRecordState::Resolving),
            );

            let status = status_from_records(&records, "game-1".to_string())
                .await
                .unwrap()
                .unwrap();

            assert_eq!(status.game_id, "game-1");
            assert_eq!(status.state, "resolving");
            assert_eq!(status.progress, 0.0);
            assert_eq!(status.downloaded_bytes, 0);
            assert_eq!(status.total_bytes, 0);
            assert_eq!(status.download_speed_bytes_per_sec, 0);
            assert_eq!(status.upload_speed_bytes_per_sec, 0);
            assert_eq!(status.peers_count, 0);
            assert!(!status.finished);
            assert_eq!(status.save_dir, "F:/Downloads/game");
            assert_eq!(status.error, None);
        });
    }
}
