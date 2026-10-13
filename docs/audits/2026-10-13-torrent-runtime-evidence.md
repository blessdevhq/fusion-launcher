# Fusion Launcher: код для первого рецензента

HEAD: `2ff0ec4505fe463583678609b0fd8efe061ba727`. Дата: 13 октября 2026 года.

Ответ на запрос `TorrentEngine`, `LibtorrentEngine::add_magnet`/stop-подобных мест и `process_next_queued_download`.

Метода `stop` в текущем TorrentEngine нет. Близкие пути — pause/resume/delete. Sidecar по stdin pause/stop завершает main loop и пытается сохранить fastresume. Подтверждение `CommandEvent::Terminated` сейчас меняет entry, но отдельный awaitable exited-сигнал отсутствует. Generation существует внутри движка, но не в manager storage callbacks.

Ниже — текущий код без изменений. Более ранние cancellation, add-task, polling и engine.delete также приведены в приложении к `2026-10-13-agent-round-2.md`.

## TorrentEngine: полный текущий trait

src-tauri/src/torrent_engine.rs, строки 1-47

```rust
//! Abstraction over the underlying P2P download engine.
//!
//! `TorrentManager` (in `torrent.rs`) owns the download state machine, SQLite
//! persistence, progress events, startup restore, and save-dir safety cleanup.
//! It must not care *how* bytes arrive. This module defines the [`TorrentEngine`]
//! trait that captures the raw torrent operations the manager needs.
//!
//! The only implementation is [`crate::libtorrent_engine::LibtorrentEngine`],
//! which drives the libtorrent sidecar. libtorrent forces MSE/PE handshake
//! obfuscation, so it slips past DPI that resets plaintext BitTorrent handshakes
//! — the reason the previous in-process librqbit engine was replaced.

use async_trait::async_trait;

/// Engine-agnostic snapshot of a torrent's live transfer stats. `progress` is a
/// fraction in `0.0..=1.0`; the manager converts it to a percentage when needed.
#[derive(Debug, Clone, Default)]
pub struct EngineTorrentStats {
    pub progress: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed_bytes_per_sec: u64,
    pub upload_speed_bytes_per_sec: u64,
    pub peers_count: usize,
    pub finished: bool,
    pub error: Option<String>,
}

/// Raw torrent operations the manager drives. Torrents are addressed by an
/// opaque numeric id the engine assigns on [`add_magnet`](TorrentEngine::add_magnet)
/// and the manager persists in `torrent_downloads.torrent_id`.
#[async_trait]
pub trait TorrentEngine: Send + Sync {
    /// Add a magnet and begin downloading into `save_dir`, returning the engine's
    /// id for the new torrent.
    async fn add_magnet(&self, magnet_uri: String, save_dir: String) -> Result<usize, String>;
    /// Pause the torrent. A no-op if the engine no longer tracks `id`.
    async fn pause(&self, id: usize) -> Result<(), String>;
    /// Resume a paused torrent. A no-op if the engine no longer tracks `id`.
    async fn resume(&self, id: usize) -> Result<(), String>;
    /// Remove the torrent; when `delete_files` is set, also delete partial data.
    async fn delete(&self, id: usize, delete_files: bool) -> Result<(), String>;
    /// Whether the engine still tracks a torrent with this id.
    fn exists(&self, id: usize) -> bool;
    /// Live stats for the torrent, or `None` if the engine no longer tracks it.
    fn stats(&self, id: usize) -> Option<EngineTorrentStats>;
}
```

## TorrentEntry: generation и состояние процесса

src-tauri/src/libtorrent_engine.rs, строки 68-97

```rust

struct TorrentEntry {
    magnet: String,
    save_dir: String,
    /// The running sidecar process, or `None` when paused/terminated.
    child: Option<CommandChild>,
    /// Generation counter incremented on each process spawn.
    generation: u64,
    /// In the middle of an asynchronous spawn operation.
    spawning: bool,
    /// In the middle of an asynchronous deletion operation.
    deleting: bool,
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
```

## Создание процесса и весь event loop

src-tauri/src/libtorrent_engine.rs, строки 348-479

```rust
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

    fn create_sidecar_process(
        &self,
        magnet: &str,
        save_dir: &str,
    ) -> Result<(tauri::async_runtime::Receiver<CommandEvent>, CommandChild), String> {
        let (rx, child) = self
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

        Ok((rx, child))
    }

    fn start_sidecar_event_loop(
        &self,
        id: usize,
        generation: u64,
        mut rx: tauri::async_runtime::Receiver<CommandEvent>,
    ) {
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
                                    let completed =
                                        entry.stats.finished || entry.stats.progress >= 1.0;
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
    }
}
```

## add_magnet, pause и resume

src-tauri/src/libtorrent_engine.rs, строки 481-655

```rust
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
                    deleting: false,
                    stats: EngineTorrentStats::default(),
                    user_stopped: false,
                    recent_stderr: Vec::new(),
                },
            );
        }

        let (rx, child) = match self.create_sidecar_process(&magnet_uri, &save_dir) {
            Ok(pair) => pair,
            Err(error) => {
                let _ = self.lock().map(|mut m| m.remove(&id));
                return Err(error);
            }
        };

        let mut map = self.lock()?;
        match map.get_mut(&id) {
            Some(entry) => {
                entry.spawning = false;
                if entry.generation == 1 && !entry.user_stopped && !entry.deleting {
                    entry.child = Some(child);
                    drop(map);
                    self.start_sidecar_event_loop(id, 1, rx);
                    Ok(id)
                } else {
                    let _ = child.kill();
                    Ok(id)
                }
            }
            None => {
                let _ = child.kill();
                Ok(id)
            }
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
            child
                .kill()
                .map_err(|e| format!("Failed to terminate torrent process on pause: {e}"))?;
        }
        Ok(())
    }

    async fn resume(&self, id: usize) -> Result<(), String> {
        // Fast early check: already running, already spawning, or currently being deleted?
        {
            let map = self.lock()?;
            let Some(entry) = map.get(&id) else {
                return Ok(());
            };
            if entry.deleting || entry.spawning || (entry.child.is_some() && !entry.user_stopped) {
                return Ok(());
            }
        }

        // Wait up to 5s (100 * 50ms) for previous child process to terminate cleanly
        for _ in 0..100 {
            let is_dead = {
                let map = self.lock()?;
                map.get(&id).is_none_or(|e| e.child.is_none())
            };
            if is_dead {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // If the old child process did not terminate within timeout, force kill it.
        // If kill fails, return error immediately, and DO NOT spawn!
        let child_to_kill = {
            let mut map = self.lock()?;
            map.get_mut(&id).and_then(|e| e.child.take())
        };
        if let Some(child) = child_to_kill {
            if let Err(e) = child.kill() {
                if let Ok(mut map) = self.lock() {
                    if let Some(entry) = map.get_mut(&id) {
                        entry.stats.error =
                            Some(format!("Failed to terminate existing sidecar: {e}"));
                    }
                }
                return Err(format!(
                    "Failed to terminate existing torrent process before resume: {e}"
                ));
            }
        }

        let (magnet, save_dir, generation) = {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            if entry.deleting || entry.spawning || (entry.child.is_some() && !entry.user_stopped) {
                return Ok(());
            }
            entry.spawning = true;
            entry.user_stopped = false;
            entry.recent_stderr.clear();
            entry.stats.error = None;
            entry.generation += 1;
            (
                entry.magnet.clone(),
                entry.save_dir.clone(),
                entry.generation,
            )
        };

        let (rx, child) = match self.create_sidecar_process(&magnet, &save_dir) {
            Ok(pair) => pair,
            Err(err) => {
                let mut map = self.lock()?;
                if let Some(e) = map.get_mut(&id) {
                    e.spawning = false;
                    e.user_stopped = true;
                    e.stats.error = Some(err.clone());
                }
                return Err(err);
            }
        };

        let mut map = self.lock()?;
        match map.get_mut(&id) {
            Some(e) => {
                e.spawning = false;
                if e.generation == generation && !e.user_stopped && !e.deleting {
                    e.child = Some(child);
                    drop(map);
                    self.start_sidecar_event_loop(id, generation, rx);
                    Ok(())
                } else {
                    let _ = child.kill();
                    Ok(())
                }
            }
            None => {
                let _ = child.kill();
                Ok(())
            }
        }
    }
```

## Текущий delete с ожиданием и очисткой

src-tauri/src/libtorrent_engine.rs, строки 657-780

```rust
    async fn delete(&self, id: usize, delete_files: bool) -> Result<(), String> {
        // 1. Signal stop to child process and mark deleting = true
        {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.deleting = true;
            entry.user_stopped = true;
            if let Some(child) = entry.child.as_mut() {
                let _ = child.write(b"{\"action\":\"stop\"}\n");
            }
        }

        // 2. Wait up to 5s (100 * 50ms) for spawning to finish and child process to exit cleanly
        for _ in 0..100 {
            let is_dead = {
                let map = self.lock()?;
                map.get(&id)
                    .is_none_or(|e| !e.spawning && e.child.is_none())
            };
            if is_dead {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // 3. If child is still present, kill it. If kill fails, reset deleting and return error
        let child_to_kill = {
            let mut map = self.lock()?;
            map.get_mut(&id).and_then(|e| e.child.take())
        };
        if let Some(child) = child_to_kill {
            if let Err(e) = child.kill() {
                if let Ok(mut map) = self.lock() {
                    if let Some(entry) = map.get_mut(&id) {
                        entry.deleting = false;
                        entry.stats.error =
                            Some(format!("Failed to kill torrent process during delete: {e}"));
                    }
                }
                return Err(format!("Failed to kill torrent process during delete: {e}"));
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
                if let Ok(mut map) = self.lock() {
                    if let Some(e) = map.get_mut(&id) {
                        e.deleting = false;
                    }
                }
                "Application state is not initialized for torrent file deletion.".to_string()
            })?;
            let library_root = crate::commands::library_root_for_app_state(&state);
            match safe_to_delete(std::path::Path::new(&save_dir), &library_root) {
                SafeDeleteOutcome::Safe(path) => {
                    if path.exists() {
                        if let Err(e) = std::fs::remove_dir_all(&path) {
                            if let Ok(mut map) = self.lock() {
                                if let Some(entry) = map.get_mut(&id) {
                                    entry.deleting = false;
                                    entry.stats.error =
                                        Some(format!("Failed to delete files: {e}"));
                                }
                            }
                            return Err(format!(
                                "Failed to delete torrent directory {}: {e}",
                                path.display()
                            ));
                        }
                    }
                }
                SafeDeleteOutcome::SafeJunction(path) => {
                    if path.exists() || is_reparse_point_or_symlink(&path) {
                        if let Err(e) = std::fs::remove_dir(&path) {
                            if let Ok(mut map) = self.lock() {
                                if let Some(entry) = map.get_mut(&id) {
                                    entry.deleting = false;
                                    entry.stats.error =
                                        Some(format!("Failed to unlink junction: {e}"));
                                }
                            }
                            return Err(format!(
                                "Failed to unlink junction {}: {e}",
                                path.display()
                            ));
                        }
                    }
                }
                SafeDeleteOutcome::Nonexistent => {}
                SafeDeleteOutcome::PolicyRefused(reason) => {
                    // Files outside library root or in forbidden directories are preserved.
                    // DO NOT block removal of the launcher record so user is not stuck.
                    eprintln!("Torrent delete files policy refusal (files preserved): {reason}");
                }
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
```

## process_next_queued_download и внутренние spawn wrappers

src-tauri/src/torrent.rs, строки 532-564

```rust
    pub async fn process_next_queued_download(&self) {
        trigger_next_queued(
            Arc::clone(&self.engine),
            Arc::clone(&self.records),
            Arc::clone(&self.store),
            self.app.clone(),
        )
        .await;
    }

    fn spawn_add_torrent_task(&self, game_id: String, magnet_uri: String, save_dir: String) {
        spawn_add_torrent_task_inner(
            Arc::clone(&self.engine),
            Arc::clone(&self.records),
            Arc::clone(&self.store),
            self.app.clone(),
            game_id,
            magnet_uri,
            save_dir,
        );
    }

    fn spawn_polling_for_game(&self, game_id: String, torrent_id: usize) {
        spawn_polling_task(
            Arc::clone(&self.engine),
            Arc::clone(&self.records),
            Arc::clone(&self.store),
            self.app.clone(),
            game_id,
            torrent_id,
        );
    }
}
```

## trigger_next_queued

src-tauri/src/torrent.rs, строки 566-631

```rust
async fn trigger_next_queued(
    engine: Arc<dyn TorrentEngine>,
    records: Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: Arc<StoreMutex<RepositoryStore>>,
    app: AppHandle,
) {
    let next_task = {
        let records_guard = records.lock().await;
        if active_download_count(&records_guard) >= MAX_ACTIVE_DOWNLOADS {
            return;
        }

        let in_memory_id = records_guard
            .values()
            .find(|r| r.state == TorrentRecordState::Queued)
            .map(|r| r.game_id.clone());

        if let Some(game_id) = in_memory_id {
            let store_guard = match lock_store(&store) {
                Ok(s) => s,
                Err(_) => return,
            };
            store_guard
                .get_torrent_download(&game_id)
                .ok()
                .flatten()
                .map(|r| (r.game_id, r.magnet_uri, r.save_dir))
        } else {
            let store_guard = match lock_store(&store) {
                Ok(s) => s,
                Err(_) => return,
            };
            store_guard
                .get_next_queued_torrent_download()
                .ok()
                .flatten()
                .map(|r| (r.game_id, r.magnet_uri, r.save_dir))
        }
    };

    let Some((game_id, magnet_uri, save_dir)) = next_task else {
        return;
    };

    if let Ok(record) =
        lock_store(&store).and_then(|s| s.set_torrent_status(&game_id, "resolving", None))
    {
        emit_download_record(&app, &record);
    }

    {
        let mut records_guard = records.lock().await;
        records_guard.insert(
            game_id.clone(),
            TorrentRecord {
                game_id: game_id.clone(),
                save_dir: save_dir.clone(),
                torrent_id: None,
                state: TorrentRecordState::Resolving,
                error: None,
            },
        );
    }

    spawn_add_torrent_task_inner(engine, records, store, app, game_id, magnet_uri, save_dir);
}
```

## Пауза и остановка на стороне Python

sidecar/torrent_sidecar.py, строки 239-318

```python
def listen_stdin(handle, is_paused_flag, stop_event):
    for line in sys.stdin:
        text = line.strip()
        if not text:
            continue
        try:
            command = json.loads(text)
            action = command.get("action")
            if action == "pause":
                handle.pause()
                is_paused_flag[0] = True
                log("info: torrent pause requested via stdin")
                stop_event.set()
                break
            elif action == "resume":
                handle.resume()
                is_paused_flag[0] = False
                log("info: torrent resumed via stdin")
            elif action == "stop":
                log("info: torrent stop requested via stdin")
                stop_event.set()
                break
        except Exception as error:
            log(f"warning: stdin command error: {error}")


def main():
    parser = argparse.ArgumentParser(
        description="Fusion Launcher libtorrent download sidecar."
    )
    parser.add_argument("magnet_link", help="magnet: URI to download")
    parser.add_argument("save_path", help="destination directory for the files")
    args = parser.parse_args()

    if not args.magnet_link.startswith("magnet:"):
        log("error: first argument must be a magnet: URI")
        return 2

    session = make_session()
    btih = extract_btih(args.magnet_link)
    resume_file = None
    if btih:
        os.makedirs(args.save_path, exist_ok=True)
        resume_file = os.path.join(args.save_path, f".{btih}.fastresume")

    handle = add_magnet_with_resume(session, args.magnet_link, args.save_path, resume_file)

    is_paused_flag = [False]
    stop_event = threading.Event()
    stdin_thread = threading.Thread(
        target=listen_stdin,
        args=(handle, is_paused_flag, stop_event),
        daemon=True
    )
    stdin_thread.start()

    last_periodic_save = time.time()
    PERIODIC_RESUME_INTERVAL_SECS = 30

    try:
        while not stop_event.is_set():
            now = time.time()
            if now - last_periodic_save >= PERIODIC_RESUME_INTERVAL_SECS:
                last_periodic_save = now
                request_save_resume(handle, only_if_modified=True)

            drain_and_save_alerts(session, resume_file)

            emit(status_payload(handle, is_paused=is_paused_flag[0]))
            time.sleep(1)

        if is_paused_flag[0]:
            emit(status_payload(handle, is_paused=True))
        save_and_wait(session, handle, resume_file)
        return 0
    except KeyboardInterrupt:
        save_and_wait(session, handle, resume_file)
        return 0


```
