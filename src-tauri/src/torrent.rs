use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StoreMutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::libtorrent_engine::LibtorrentEngine;
use crate::schema::TorrentDownloadRecord;
use crate::storage::RepositoryStore;
use crate::torrent_engine::{EngineTorrentStats, TorrentEngine};
use crate::AppState;

pub(crate) const MAX_ACTIVE_DOWNLOADS: usize = 1;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(test)]
const ACTIVE_DOWNLOAD_LIMIT_ERROR: &str =
    "Another torrent download is already active. Please wait for it to finish before starting a new one.";

#[derive(Clone)]
pub struct TorrentManager {
    engine: Arc<dyn TorrentEngine>,
    records: Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: Arc<StoreMutex<RepositoryStore>>,
    events: Arc<dyn DownloadEventSink>,
    operations: OperationCoordinator,
    owners: Arc<StoreMutex<HashMap<String, OperationLease>>>,
    tasks: Arc<StoreMutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    control: Arc<Mutex<()>>,
}

#[derive(Clone)]
struct TorrentRecord {
    game_id: String,
    save_dir: String,
    torrent_id: Option<usize>,
    state: TorrentRecordState,
    error: Option<String>,
    identity: DownloadAttempt,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TorrentRecordState {
    Queued,
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
            Self::Queued => "queued",
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
    pub attempt_id: Option<String>,
    pub run_id: Option<u64>,
    pub subject_type: Option<String>,
    pub display_name: Option<String>,
    pub parent_game_id: Option<String>,
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

include!("torrent_runtime.rs");
#[cfg(test)]
#[path = "torrent_runtime_tests.rs"]
mod runtime_tests;

#[tauri::command]
pub async fn start_magnet_download(
    game_id: String,
    magnet_uri: String,
    save_dir: String,
    state: State<'_, AppState>,
) -> Result<TorrentStartReport, String> {
    state
        .torrents()?
        .start_magnet_download(game_id, magnet_uri, save_dir)
        .await
}

#[tauri::command]
pub async fn get_torrent_status(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<TorrentStatus, String> {
    state.torrents()?.get_torrent_status(game_id).await
}

#[tauri::command]
pub fn get_game_download(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Option<TorrentDownloadRecord>, String> {
    if let Some(torrents) = state.torrents.as_ref() {
        return torrents.get_game_download(&game_id);
    }
    lock_store(&state.store)?.get_torrent_download(&game_id)
}

#[tauri::command]
pub fn list_torrent_downloads(
    state: State<'_, AppState>,
) -> Result<Vec<TorrentDownloadRecord>, String> {
    if let Some(torrents) = state.torrents.as_ref() {
        return torrents.list_downloads();
    }
    lock_store(&state.store)?.list_torrent_downloads()
}

#[tauri::command]
pub async fn pause_download(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<TorrentDownloadRecord, String> {
    state.torrents()?.pause_download(game_id).await
}

#[tauri::command]
pub async fn resume_download(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<TorrentDownloadRecord, String> {
    state.torrents()?.resume_download(game_id).await
}

#[tauri::command]
pub async fn cancel_download(
    game_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<TorrentDownloadRecord, String> {
    let record = lock_store(&state.store)?
        .get_torrent_download(&game_id)?
        .ok_or_else(|| format!("Unknown torrent download: {game_id}"))?;
    if is_direct_download_record(&record) {
        return cancel_direct_download(game_id, state.inner(), &app, record).await;
    }

    state.torrents()?.cancel_download(game_id).await
}

async fn cancel_direct_download(
    game_id: String,
    state: &AppState,
    app: &AppHandle,
    _record: TorrentDownloadRecord,
) -> Result<TorrentDownloadRecord, String> {
    let ticket = state.direct_downloads.begin_delete(&game_id)?;
    let latest = lock_store(&state.store)?
        .get_torrent_download(&game_id)?
        .ok_or("Download missing.")?;
    if latest.status == "completed" {
        return Err("Completed downloads cannot be cancelled.".into());
    }
    let cancelling = lock_store(&state.store)?.set_torrent_status(&game_id, "cancelling", None)?;
    emit_download_record(app, &cancelling);
    ticket.wait_owner(Duration::from_secs(20)).await?;
    let cancelled = lock_store(&state.store)?.set_torrent_status(&game_id, "cancelled", None)?;
    emit_download_record(app, &cancelled);
    Ok(cancelled)
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

#[cfg(test)]
fn ensure_can_track_game(
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

    Ok(())
}

#[cfg(test)]
fn ensure_can_start_download(
    records: &HashMap<String, TorrentRecord>,
    game_id: &str,
) -> Result<(), String> {
    ensure_can_track_game(records, game_id)?;

    if active_download_count(records) >= MAX_ACTIVE_DOWNLOADS {
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

fn record_blocks_new_download(record: &TorrentRecord) -> bool {
    matches!(
        record.state,
        TorrentRecordState::Resolving
            | TorrentRecordState::Downloading
            | TorrentRecordState::Cancelling
    )
}

async fn status_from_records(
    engine: &Arc<dyn TorrentEngine>,
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

    // While the magnet is still resolving, the engine has no id yet (or no stats),
    // so report a zero-progress pending status rather than a missing torrent.
    let Some(stats) = record.torrent_id.and_then(|id| engine.stats(id)) else {
        return Ok(Some(pending_status(record)));
    };

    Ok(Some(status_from_stats(&record, &stats)))
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

fn status_from_stats(record: &TorrentRecord, stats: &EngineTorrentStats) -> TorrentStatus {
    let finished = stats.finished || stats.progress >= 1.0;
    let error = stats.error.clone().or_else(|| record.error.clone());
    let state = if error.is_some() {
        TorrentRecordState::Error.as_str()
    } else if finished {
        TorrentRecordState::Completed.as_str()
    } else {
        record.state.as_str()
    };

    TorrentStatus {
        game_id: record.game_id.clone(),
        state: state.to_string(),
        progress: clamp_progress(stats.progress),
        downloaded_bytes: stats.downloaded_bytes,
        total_bytes: stats.total_bytes,
        download_speed_bytes_per_sec: stats.download_speed_bytes_per_sec,
        upload_speed_bytes_per_sec: stats.upload_speed_bytes_per_sec,
        peers_count: stats.peers_count,
        finished,
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

fn event_from_status(status: &TorrentStatus) -> DownloadProgressEvent {
    DownloadProgressEvent {
        game_id: status.game_id.clone(),
        attempt_id: None,
        run_id: None,
        subject_type: None,
        display_name: None,
        parent_game_id: None,
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
    let mut event = event_from_status(&status);
    event.subject_type = record.subject_type.clone();
    event.display_name = record.display_name.clone();
    event.parent_game_id = record.parent_game_id.clone();
    event.attempt_id = record.attempt_id.clone();
    event.run_id = record.run_id;
    let _ = app.emit("download:progress", event);
}

fn lock_store(
    store: &Arc<StoreMutex<RepositoryStore>>,
) -> Result<std::sync::MutexGuard<'_, RepositoryStore>, String> {
    store
        .lock()
        .map_err(|_| "Repository store lock is poisoned.".to_string())
}

fn is_direct_download_record(record: &TorrentDownloadRecord) -> bool {
    record.magnet_uri.starts_with("direct:")
}

fn clamp_progress(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }

    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Engine that tracks nothing — enough to exercise the manager's
    /// resolving/missing-torrent paths without a real session.
    struct NoopEngine;

    #[async_trait::async_trait]
    impl TorrentEngine for NoopEngine {
        async fn stop(&self, _: usize) -> Result<(), String> {
            Ok(())
        }
        async fn stop_for_path(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn add_magnet(&self, _: String, _: String) -> Result<usize, String> {
            Err("not supported in tests".to_string())
        }
        async fn pause(&self, _: usize) -> Result<(), String> {
            Ok(())
        }
        async fn resume(&self, _: usize) -> Result<(), String> {
            Ok(())
        }
        async fn delete(&self, _: usize, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn exists(&self, _: usize) -> bool {
            false
        }
        fn stats(&self, _: usize) -> Option<EngineTorrentStats> {
            None
        }
    }

    fn noop_engine() -> Arc<dyn TorrentEngine> {
        Arc::new(NoopEngine)
    }

    fn test_record(game_id: &str, state: TorrentRecordState) -> TorrentRecord {
        TorrentRecord {
            identity: DownloadAttempt {
                subject: game_id.into(),
                attempt: "test".into(),
                run: 1,
            },
            game_id: game_id.to_string(),
            save_dir: "F:/Downloads/game".to_string(),
            torrent_id: None,
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
    fn queued_record_blocks_duplicate_tracking() {
        let mut records = HashMap::new();
        records.insert(
            "game-1".to_string(),
            test_record("game-1", TorrentRecordState::Queued),
        );

        assert_eq!(
            ensure_can_track_game(&records, "game-1").unwrap_err(),
            "Torrent already tracked for game: game-1"
        );
    }

    #[test]
    fn queued_record_does_not_count_towards_active_download_limit() {
        let mut records = HashMap::new();
        records.insert(
            "game-1".to_string(),
            test_record("game-1", TorrentRecordState::Queued),
        );

        assert!(ensure_can_start_download(&records, "game-2").is_ok());
    }

    #[test]
    fn unknown_game_status_returns_none() {
        let engine = noop_engine();
        let records = Arc::new(Mutex::new(HashMap::new()));
        let status = tauri::async_runtime::block_on(status_from_records(
            &engine,
            &records,
            "missing-game".to_string(),
        ))
        .unwrap();

        assert!(status.is_none());
    }

    #[test]
    fn resolving_record_reports_zero_progress() {
        let engine = noop_engine();
        let records = Arc::new(Mutex::new(HashMap::new()));
        tauri::async_runtime::block_on(async {
            records.lock().await.insert(
                "game-1".to_string(),
                test_record("game-1", TorrentRecordState::Resolving),
            );

            let status = status_from_records(&engine, &records, "game-1".to_string())
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
