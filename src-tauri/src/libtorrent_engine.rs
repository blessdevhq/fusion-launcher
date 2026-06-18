//! [`TorrentEngine`] backed by the libtorrent Python sidecar (`sidecar/`).
//!
//! Unlike librqbit, libtorrent forces MSE/PE handshake obfuscation, so it slips
//! past DPI that resets plaintext BitTorrent handshakes. The sidecar is a
//! one-process-per-download executable: it takes `<magnet> <save_path>` and
//! streams one JSON status line per second on stdout
//! (`{"progress","speed_bytes","peers","state"}`). This engine adapts that model
//! onto the multi-torrent [`TorrentEngine`] contract:
//!
//! * `add_magnet` spawns a sidecar process and assigns it an id.
//! * `pause` kills the process but keeps the entry; `resume` respawns it, which
//!   re-checks the partial files already on disk and continues.
//! * `delete` removes the entry and kills the process; the manager deletes files.
//! * `stats` returns the latest parsed status line.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

use crate::torrent_engine::{EngineTorrentStats, TorrentEngine};

/// Name of the bundled sidecar binary (Tauri `externalBin`, resolved per target
/// triple at runtime).
const SIDECAR_NAME: &str = "fusion-torrent";

/// One status line emitted by the sidecar on stdout. `progress`/`speed_bytes`/
/// `peers`/`state` plus `downloaded_bytes`/`total_bytes`; all but `progress` are
/// defaulted so older sidecar builds that omit a field still parse.
#[derive(Debug, Clone, Deserialize)]
struct SidecarProgress {
    progress: f64,
    #[serde(default)]
    speed_bytes: u64,
    #[serde(default)]
    peers: usize,
    #[serde(default)]
    state: String,
    #[serde(default)]
    downloaded_bytes: u64,
    #[serde(default)]
    total_bytes: u64,
}

/// Parse a single JSON status line into engine stats, or `None` if the line is
/// not valid JSON (e.g. a stray diagnostic that slipped onto stdout).
fn parse_progress_line(line: &str) -> Option<EngineTorrentStats> {
    let parsed: SidecarProgress = serde_json::from_str(line.trim()).ok()?;
    let progress = (parsed.progress / 100.0).clamp(0.0, 1.0);
    let finished = matches!(parsed.state.as_str(), "finished" | "seeding") || progress >= 1.0;
    Some(EngineTorrentStats {
        progress,
        downloaded_bytes: parsed.downloaded_bytes,
        total_bytes: parsed.total_bytes,
        download_speed_bytes_per_sec: parsed.speed_bytes,
        upload_speed_bytes_per_sec: 0,
        peers_count: parsed.peers,
        finished,
        error: None,
    })
}

struct TorrentEntry {
    magnet: String,
    save_dir: String,
    /// The running sidecar process, or `None` when paused/terminated.
    child: Option<CommandChild>,
    /// Latest stats parsed from the sidecar's stdout.
    stats: EngineTorrentStats,
    /// Set when the manager paused/cancelled this torrent, so the process's
    /// termination is treated as intentional rather than a crash.
    user_stopped: bool,
}

pub struct LibtorrentEngine {
    app: AppHandle,
    next_id: AtomicUsize,
    torrents: Arc<Mutex<HashMap<usize, TorrentEntry>>>,
}

impl LibtorrentEngine {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            next_id: AtomicUsize::new(1),
            torrents: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<usize, TorrentEntry>>, String> {
        self.torrents
            .lock()
            .map_err(|_| "Torrent engine state is poisoned.".to_string())
    }

    /// Spawn the sidecar for `id` and stream its stdout into the shared map.
    /// The entry for `id` must already exist so the reader can update it.
    fn spawn_sidecar(
        &self,
        id: usize,
        magnet: &str,
        save_dir: &str,
    ) -> Result<CommandChild, String> {
        let (mut rx, child) = self
            .app
            .shell()
            .sidecar(SIDECAR_NAME)
            .map_err(|error| format!("Failed to locate torrent sidecar: {error}"))?
            .args([magnet, save_dir])
            .spawn()
            .map_err(|error| format!("Failed to start torrent sidecar: {error}"))?;

        let torrents = Arc::clone(&self.torrents);
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) => {
                        let line = String::from_utf8_lossy(&bytes);
                        if let Some(stats) = parse_progress_line(&line) {
                            if let Ok(mut map) = torrents.lock() {
                                if let Some(entry) = map.get_mut(&id) {
                                    entry.stats = stats;
                                }
                            }
                        }
                    }
                    CommandEvent::Error(message) => {
                        if let Ok(mut map) = torrents.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                entry.stats.error =
                                    Some(format!("Torrent sidecar error: {message}"));
                            }
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        if let Ok(mut map) = torrents.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                entry.child = None;
                                let completed = entry.stats.finished || entry.stats.progress >= 1.0;
                                if completed {
                                    entry.stats.finished = true;
                                } else if !entry.user_stopped && payload.code != Some(0) {
                                    entry.stats.error = Some(format!(
                                        "Torrent sidecar exited unexpectedly (code {:?}).",
                                        payload.code
                                    ));
                                }
                            }
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(child)
    }
}

#[async_trait]
impl TorrentEngine for LibtorrentEngine {
    async fn add_magnet(&self, magnet_uri: String, save_dir: String) -> Result<usize, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        // Insert the entry before spawning so the reader task can find it even if
        // the process terminates immediately.
        {
            let mut map = self.lock()?;
            map.insert(
                id,
                TorrentEntry {
                    magnet: magnet_uri.clone(),
                    save_dir: save_dir.clone(),
                    child: None,
                    stats: EngineTorrentStats::default(),
                    user_stopped: false,
                },
            );
        }

        match self.spawn_sidecar(id, &magnet_uri, &save_dir) {
            Ok(child) => {
                if let Some(entry) = self.lock()?.get_mut(&id) {
                    entry.child = Some(child);
                }
                Ok(id)
            }
            Err(error) => {
                self.lock()?.remove(&id);
                Err(error)
            }
        }
    }

    async fn pause(&self, id: usize) -> Result<(), String> {
        let child = {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.user_stopped = true;
            entry.child.take()
        };
        if let Some(child) = child {
            let _ = child.kill();
        }
        Ok(())
    }

    async fn resume(&self, id: usize) -> Result<(), String> {
        let (magnet, save_dir) = {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.user_stopped = false;
            (entry.magnet.clone(), entry.save_dir.clone())
        };

        let child = self.spawn_sidecar(id, &magnet, &save_dir)?;
        if let Some(entry) = self.lock()?.get_mut(&id) {
            entry.child = Some(child);
        }
        Ok(())
    }

    async fn delete(&self, id: usize, _delete_files: bool) -> Result<(), String> {
        // The manager removes the files via `remove_save_dir_if_safe`; here we
        // just stop and forget the process.
        let entry = self.lock()?.remove(&id);
        if let Some(mut entry) = entry {
            entry.user_stopped = true;
            if let Some(child) = entry.child.take() {
                let _ = child.kill();
            }
        }
        Ok(())
    }

    fn exists(&self, id: usize) -> bool {
        self.lock()
            .map(|map| map.contains_key(&id))
            .unwrap_or(false)
    }

    fn stats(&self, id: usize) -> Option<EngineTorrentStats> {
        self.lock().ok()?.get(&id).map(|entry| entry.stats.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_downloading_line() {
        let stats = parse_progress_line(
            r#"{"progress": 42.5, "speed_bytes": 1048576, "peers": 15, "state": "downloading"}"#,
        )
        .expect("valid line should parse");
        assert!((stats.progress - 0.425).abs() < 1e-9);
        assert_eq!(stats.download_speed_bytes_per_sec, 1_048_576);
        assert_eq!(stats.peers_count, 15);
        assert!(!stats.finished);
        assert_eq!(stats.error, None);
    }

    #[test]
    fn seeding_and_finished_states_are_finished() {
        for state in ["seeding", "finished"] {
            let line = format!(
                r#"{{"progress": 100.0, "speed_bytes": 0, "peers": 1, "state": "{state}"}}"#
            );
            assert!(parse_progress_line(&line).unwrap().finished);
        }
    }

    #[test]
    fn checking_state_is_not_finished() {
        let stats = parse_progress_line(
            r#"{"progress": 0.0, "speed_bytes": 0, "peers": 0, "state": "checking"}"#,
        )
        .unwrap();
        assert!(!stats.finished);
        assert_eq!(stats.progress, 0.0);
    }

    #[test]
    fn progress_is_clamped_to_unit_range() {
        let over = parse_progress_line(
            r#"{"progress": 150.0, "speed_bytes": 0, "peers": 0, "state": "downloading"}"#,
        )
        .unwrap();
        assert_eq!(over.progress, 1.0);
        assert!(over.finished);
    }

    #[test]
    fn optional_byte_totals_are_parsed_when_present() {
        let stats = parse_progress_line(
            r#"{"progress": 10.0, "speed_bytes": 1, "peers": 2, "state": "downloading", "downloaded_bytes": 500, "total_bytes": 5000}"#,
        )
        .unwrap();
        assert_eq!(stats.downloaded_bytes, 500);
        assert_eq!(stats.total_bytes, 5000);
    }

    #[test]
    fn non_json_lines_are_ignored() {
        assert!(parse_progress_line("starting up DHT...").is_none());
        assert!(parse_progress_line("").is_none());
    }
}
