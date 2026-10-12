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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use serde::Deserialize;
use tauri::{AppHandle, Manager};
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
    /// Generation counter incremented on each process spawn.
    generation: u64,
    /// In the middle of an asynchronous spawn operation.
    spawning: bool,
    /// Latest stats parsed from the sidecar's stdout.
    stats: EngineTorrentStats,
    /// Set when the manager paused/cancelled this torrent, so the process's
    /// termination is treated as intentional rather than a crash.
    user_stopped: bool,
    /// Tail of the sidecar's stderr (diagnostics, e.g. `fatal: ...` lines). Kept
    /// so an unexpected exit can surface the real Python error instead of a bare
    /// exit code.
    recent_stderr: Vec<String>,
}

/// Keep only the last few stderr lines so a crash report stays readable.
const MAX_STDERR_LINES: usize = 8;

const SIDECAR_DEPENDENCY_HINT: &str = "Torrent sidecar could not load libtorrent. Install or repair the Microsoft Visual C++ 2015-2022 x64 Redistributable, then reinstall Fusion Launcher from the bundled Windows installer.";

pub(crate) fn sidecar_dependency_error_message(error: &str) -> Option<&'static str> {
    let normalized = error.to_ascii_lowercase();
    let import_failure = normalized.contains("importerror")
        || normalized.contains("dll load failed")
        || normalized.contains("failed to execute script");
    let sidecar_failure =
        normalized.contains("libtorrent") || normalized.contains("torrent_sidecar");
    if import_failure && sidecar_failure {
        Some(SIDECAR_DEPENDENCY_HINT)
    } else {
        None
    }
}

fn decorate_sidecar_error(error: &str) -> String {
    match sidecar_dependency_error_message(error) {
        Some(hint) => format!("{error} {hint}"),
        None => error.to_string(),
    }
}

fn is_valid_btih_hash(hash: &str) -> bool {
    let len = hash.len();
    if len == 40 {
        hash.chars().all(|c| c.is_ascii_hexdigit())
    } else if len == 32 {
        hash.chars()
            .all(|c| matches!(c, '2'..='7' | 'a'..='z' | 'A'..='Z'))
    } else {
        false
    }
}

fn extract_btih(magnet: &str) -> Option<&str> {
    let prefix = "xt=urn:btih:";
    for part in magnet.split('&') {
        let part = part
            .strip_prefix("magnet:?")
            .or_else(|| part.strip_prefix("MAGNET:?"))
            .unwrap_or(part);
        let Some(candidate) = part.get(..prefix.len()) else {
            continue;
        };
        if candidate.eq_ignore_ascii_case(prefix) {
            let Some(hash) = part.get(prefix.len()..).map(str::trim) else {
                continue;
            };
            if is_valid_btih_hash(hash) {
                return Some(hash);
            }
        }
    }
    None
}

fn safe_to_delete(dir: &Path, library_root: &Path) -> Result<PathBuf, String> {
    if !dir.exists() {
        return Ok(dir.to_path_buf());
    }
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize delete path {}: {e}", dir.display()))?;
    let root = library_root
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize library root {}: {e}", library_root.display()))?;
    if dir == root || dir.parent() == Some(&root) || !dir.starts_with(&root) {
        return Err(format!(
            "Refusing to delete root, parent category, or external directory: {}",
            dir.display()
        ));
    }
    let rel = dir
        .strip_prefix(&root)
        .map_err(|_| format!("Path {} is outside library root {}", dir.display(), root.display()))?;
    let components: Vec<_> = rel.iter().map(|c| c.to_string_lossy().to_string()).collect();
    if components.len() < 2 {
        return Err(format!(
            "Delete path must be at least 2 levels deep inside library root (<category>/<game>): {}",
            dir.display()
        ));
    }
    let top_category = components[0].to_ascii_lowercase();
    const FORBIDDEN_TOP_CATEGORIES: &[&str] = &[
        "emulators",
        "system",
        "bios",
        "firmware",
        "shared-cache",
        "cache",
        "config",
        "profiles",
    ];
    if FORBIDDEN_TOP_CATEGORIES.contains(&top_category.as_str()) {
        return Err(format!(
            "Refusing to delete inside reserved system/emulator category '{}': {}",
            top_category,
            dir.display()
        ));
    }
    Ok(dir)
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
        generation: u64,
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

        if let Err(error) = crate::job::bind_to_launcher_lifetime(child.pid()) {
            let _ = child.kill();
            return Err(format!(
                "Failed to bind torrent sidecar to Windows Job Object (fail-closed): {error}"
            ));
        }

        let torrents = Arc::clone(&self.torrents);
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) => {
                        let line = String::from_utf8_lossy(&bytes);
                        if let Some(stats) = parse_progress_line(&line) {
                            if let Ok(mut map) = torrents.lock() {
                                if let Some(entry) = map.get_mut(&id) {
                                    if entry.generation == generation {
                                        entry.stats = stats;
                                    }
                                }
                            }
                        }
                    }
                    CommandEvent::Stderr(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        if let Ok(mut map) = torrents.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                if entry.generation == generation {
                                    for line in text.lines() {
                                        let line = line.trim();
                                        if !line.is_empty() {
                                            entry.recent_stderr.push(line.to_string());
                                        }
                                    }
                                    let overflow =
                                        entry.recent_stderr.len().saturating_sub(MAX_STDERR_LINES);
                                    if overflow > 0 {
                                        entry.recent_stderr.drain(0..overflow);
                                    }
                                }
                            }
                        }
                    }
                    CommandEvent::Error(message) => {
                        if let Ok(mut map) = torrents.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                if entry.generation == generation {
                                    entry.stats.error =
                                        Some(format!("Torrent sidecar error: {message}"));
                                }
                            }
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        if let Ok(mut map) = torrents.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                // If this termination belongs to an older process run, ignore it.
                                if entry.generation != generation {
                                    break;
                                }
                                entry.child = None;
                                if entry.user_stopped {
                                    entry.stats.download_speed_bytes_per_sec = 0;
                                    entry.stats.upload_speed_bytes_per_sec = 0;
                                } else {
                                    let completed = entry.stats.finished || entry.stats.progress >= 1.0;
                                    if completed {
                                        entry.stats.finished = true;
                                        entry.stats.download_speed_bytes_per_sec = 0;
                                        entry.stats.upload_speed_bytes_per_sec = 0;
                                    } else {
                                        let detail = if entry.recent_stderr.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" {}", entry.recent_stderr.join(" | "))
                                        };
                                        let error = format!(
                                            "Torrent sidecar exited unexpectedly (code {:?}).{detail}",
                                            payload.code
                                        );
                                        entry.stats.error = Some(decorate_sidecar_error(&error));
                                    }
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

        {
            let mut map = self.lock()?;
            map.insert(
                id,
                TorrentEntry {
                    magnet: magnet_uri.clone(),
                    save_dir: save_dir.clone(),
                    child: None,
                    generation: 1,
                    spawning: true,
                    stats: EngineTorrentStats::default(),
                    user_stopped: false,
                    recent_stderr: Vec::new(),
                },
            );
        }

        let result = self.spawn_sidecar(id, 1, &magnet_uri, &save_dir);
        let mut map = self.lock()?;
        match (map.get_mut(&id), result) {
            (Some(entry), Ok(child)) => {
                entry.spawning = false;
                if entry.generation == 1 && !entry.user_stopped {
                    entry.child = Some(child);
                } else {
                    let _ = child.kill();
                }
                Ok(id)
            }
            (Some(_), Err(error)) => {
                map.remove(&id);
                Err(error)
            }
            (None, Ok(child)) => {
                let _ = child.kill();
                Ok(id)
            }
            (None, Err(error)) => Err(error),
        }
    }

    async fn pause(&self, id: usize) -> Result<(), String> {
        let child_to_kill = {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.user_stopped = true;
            if let Some(child) = entry.child.as_mut() {
                if child.write(b"{\"action\":\"pause\"}\n").is_ok() {
                    entry.stats.download_speed_bytes_per_sec = 0;
                    None
                } else {
                    entry.child.take()
                }
            } else {
                None
            }
        };
        if let Some(child) = child_to_kill {
            child.kill().map_err(|e| format!("Failed to terminate torrent process on pause: {e}"))?;
        }
        Ok(())
    }

    async fn resume(&self, id: usize) -> Result<(), String> {
        // Fast early check: already running or already spawning?
        {
            let map = self.lock()?;
            let Some(entry) = map.get(&id) else {
                return Ok(());
            };
            if entry.spawning || (entry.child.is_some() && !entry.user_stopped) {
                return Ok(());
            }
        }

        // Wait up to 5s (100 * 50ms) for previous child process to terminate cleanly
        for _ in 0..100 {
            let is_dead = {
                let map = self.lock()?;
                map.get(&id).map_or(true, |e| e.child.is_none())
            };
            if is_dead {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // If the old child process did not terminate within timeout, force kill it
        {
            let mut map = self.lock()?;
            if let Some(entry) = map.get_mut(&id) {
                if let Some(child) = entry.child.take() {
                    child.kill().map_err(|e| {
                        format!("Failed to terminate existing torrent process before resume: {e}")
                    })?;
                }
            }
        }

        let (magnet, save_dir, generation) = {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            if entry.spawning || (entry.child.is_some() && !entry.user_stopped) {
                return Ok(());
            }
            entry.spawning = true;
            entry.user_stopped = false;
            entry.recent_stderr.clear();
            entry.stats.error = None;
            entry.generation += 1;
            (entry.magnet.clone(), entry.save_dir.clone(), entry.generation)
        };

        let spawn_result = self.spawn_sidecar(id, generation, &magnet, &save_dir);
        let mut map = self.lock()?;
        match (map.get_mut(&id), spawn_result) {
            (Some(e), Ok(child)) => {
                e.spawning = false;
                if e.generation == generation && !e.user_stopped {
                    e.child = Some(child);
                } else {
                    let _ = child.kill();
                }
                Ok(())
            }
            (Some(e), Err(err)) => {
                e.spawning = false;
                e.user_stopped = true;
                e.stats.error = Some(err.clone());
                Err(err)
            }
            (None, Ok(child)) => {
                let _ = child.kill();
                Ok(())
            }
            (None, Err(err)) => Err(err),
        }
    }

    async fn delete(&self, id: usize, delete_files: bool) -> Result<(), String> {
        // 1. Signal stop to child process
        {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.user_stopped = true;
            if let Some(child) = entry.child.as_mut() {
                let _ = child.write(b"{\"action\":\"stop\"}\n");
            }
        }

        // 2. Wait up to 5s (100 * 50ms) for child process to exit cleanly
        for _ in 0..100 {
            let is_dead = {
                let map = self.lock()?;
                map.get(&id).map_or(true, |e| e.child.is_none())
            };
            if is_dead {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // 3. If child is still present, kill it. If kill fails, return error immediately
        // WITHOUT removing the record or deleting files!
        {
            let mut map = self.lock()?;
            if let Some(entry) = map.get_mut(&id) {
                if let Some(child) = entry.child.take() {
                    child.kill().map_err(|e| {
                        format!("Failed to kill torrent process during delete: {e}")
                    })?;
                }
            }
        }

        // 4. Brief delay to allow Windows file locks to be fully released
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // 5. Get entry info WITHOUT removing from map yet!
        let (magnet, save_dir) = {
            let map = self.lock()?;
            let Some(entry) = map.get(&id) else {
                return Ok(());
            };
            (entry.magnet.clone(), entry.save_dir.clone())
        };

        // 6. If delete_files, validate and delete files first
        if delete_files {
            let state = self.app.try_state::<crate::AppState>().ok_or_else(|| {
                "Application state is not initialized for torrent file deletion.".to_string()
            })?;
            let library_root = crate::commands::library_root_for_app_state(&state);
            let safe_dir = safe_to_delete(std::path::Path::new(&save_dir), &library_root)?;
            if safe_dir.exists() {
                std::fs::remove_dir_all(&safe_dir).map_err(|e| {
                    format!("Failed to delete torrent directory {}: {e}", safe_dir.display())
                })?;
            }
        }

        // 7. Delete fastresume files
        if let Some(btih) = extract_btih(&magnet) {
            let save_path = std::path::Path::new(&save_dir);
            let _ = std::fs::remove_file(save_path.join(format!(".{btih}.fastresume")));
            let _ = std::fs::remove_file(save_path.join(format!(".{btih}.fastresume.tmp")));
        }

        // 8. ONLY remove entry after all deletion operations succeeded!
        self.lock()?.remove(&id);

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

    #[test]
    fn classifies_libtorrent_dll_import_failure() {
        let error = "Torrent sidecar exited unexpectedly (code Some(1)). Traceback | ImportError: DLL load failed while importing libtorrent: missing module. | [PYI-1:ERROR] Failed to execute script 'torrent_sidecar'";

        let message = sidecar_dependency_error_message(error).unwrap();

        assert!(message.contains("Visual C++"));
        assert!(decorate_sidecar_error(error).contains("Visual C++"));
    }

    #[test]
    fn leaves_unrelated_sidecar_errors_unchanged() {
        let error = "Torrent sidecar exited unexpectedly (code Some(1)). No peers found";

        assert_eq!(sidecar_dependency_error_message(error), None);
        assert_eq!(decorate_sidecar_error(error), error);
    }

    #[test]
    fn extracts_btih_from_magnet_link() {
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=test"),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        // Base32 32-character hash
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:4w53q6r7stuvwxyz234567abcdefghij&dn=test"),
            Some("4w53q6r7stuvwxyz234567abcdefghij")
        );
        // Uppercase scheme prefix
        assert_eq!(
            extract_btih("MAGNET:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=test"),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        // Cyrillic title before xt: must not panic and must extract valid hash
        assert_eq!(
            extract_btih("magnet:?dn=Играть в игры&xt=urn:btih:0123456789abcdef0123456789abcdef01234567"),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        // Cyrillic title after xt
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Играть в игры"),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        // Cyrillic parameter name
        assert_eq!(
            extract_btih("magnet:?название=тест&xt=urn:btih:4w53q6r7stuvwxyz234567abcdefghij"),
            Some("4w53q6r7stuvwxyz234567abcdefghij")
        );
        assert_eq!(extract_btih("https://example.com/file.zip"), None);
        assert_eq!(extract_btih(""), None);
    }

    #[test]
    fn extract_btih_rejects_path_traversal_and_malformed_hashes() {
        // Path traversal attempts
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:../../../../windows/system32&dn=attack"),
            None
        );
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:..\\..\\windows\\system32&dn=attack"),
            None
        );
        // Hash with slash or backslash
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:0123456789abcdef/0123456789abcdef0123456&dn=attack"),
            None
        );
        // Length invalid (too short)
        assert_eq!(extract_btih("magnet:?xt=urn:btih:0123456789abcdef&dn=short"), None);
        // Length invalid (too long)
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567890&dn=long"),
            None
        );
        // Non-hex chars in 40-char string
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef0123456g&dn=bad"),
            None
        );
    }

    #[test]
    fn safe_to_delete_accepts_subdirectories_and_protects_siblings_and_parents() {
        let temp = tempfile::tempdir().unwrap();
        let lib_root = temp.path().join("library");
        let downloads_category = lib_root.join("downloads");
        let game1_dir = downloads_category.join("game1");
        let game2_dir = downloads_category.join("game2");
        let outside_dir = temp.path().join("other_data");

        std::fs::create_dir_all(&game1_dir).unwrap();
        std::fs::create_dir_all(&game2_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();

        std::fs::write(game1_dir.join("rom.nes"), b"game1").unwrap();
        std::fs::write(game2_dir.join("rom.nes"), b"game2").unwrap();

        // 1. Valid subdirectory inside library root is accepted
        let canonical_game1 = safe_to_delete(&game1_dir, &lib_root).expect("game1 subfolder should be safe");
        assert!(canonical_game1.ends_with("game1"));

        // Simulate deleting game1
        std::fs::remove_dir_all(&canonical_game1).unwrap();
        assert!(!game1_dir.exists());
        // Verify game2 is completely untouched!
        assert!(game2_dir.exists());
        assert_eq!(std::fs::read(game2_dir.join("rom.nes")).unwrap(), b"game2");

        // 2. Refuses to delete the entire library root itself
        assert!(safe_to_delete(&lib_root, &lib_root).is_err());

        // 3. Refuses to delete top-level category (e.g. library/downloads) which would wipe all games
        assert!(safe_to_delete(&downloads_category, &lib_root).is_err());

        // 4. Refuses to delete path outside library root
        assert!(safe_to_delete(&outside_dir, &lib_root).is_err());

        // 5. Nonexistent path is a safe no-op (nothing to delete)
        assert!(safe_to_delete(&downloads_category.join("already_deleted"), &lib_root).is_ok());

        // 6. Refuses to delete paths in reserved directories (shared-cache, emulators, etc.)
        let shared_cache_dir = lib_root.join("shared-cache").join("temporary");
        std::fs::create_dir_all(&shared_cache_dir).unwrap();
        assert!(safe_to_delete(&shared_cache_dir, &lib_root).is_err());

        let emulators_dir = lib_root.join("emulators").join("snes");
        std::fs::create_dir_all(&emulators_dir).unwrap();
        assert!(safe_to_delete(&emulators_dir, &lib_root).is_err());
    }
}
