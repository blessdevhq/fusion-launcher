//! [`TorrentEngine`] backed by the libtorrent Python sidecar (`sidecar/`).
//!
//! Unlike librqbit, libtorrent forces MSE/PE handshake obfuscation, so it slips
//! past DPI that resets plaintext BitTorrent handshakes. The sidecar is a
//! one-invocation-per-download executable (PyInstaller onefile also starts a
//! Python child): it takes `<magnet> <save_path>` and
//! streams one JSON status line per second on stdout
//! (`{"progress","speed_bytes","peers","state"}`). This engine adapts that model
//! onto the multi-torrent [`TorrentEngine`] contract:
//!
//! * `add_magnet` spawns a sidecar process and assigns it an id.
//! * `pause` stops the process but keeps the entry; `resume` respawns it, which
//!   re-checks the partial files already on disk and continues.
//! * `delete` removes the entry and kills the process; the manager deletes files.
//! * `stats` returns the latest parsed status line.

use std::collections::HashMap;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(windows)]
use crate::windows_sidecar::WindowsChild as CommandChild;
use async_trait::async_trait;
use serde::Deserialize;
use tauri::AppHandle;
#[cfg(not(windows))]
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::process::CommandEvent;
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
    #[serde(default)]
    inventory: Option<Vec<String>>,
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
        inventory: parsed.inventory,
    })
}

struct TorrentEntry {
    magnet: String,
    save_dir: String,
    save_identity: String,
    /// The running sidecar process, or `None` when paused/terminated.
    child: Option<CommandChild>,
    /// Generation counter incremented on each process spawn.
    generation: u64,
    /// Serializes lifecycle operations without holding a std mutex across await.
    operation: Arc<tokio::sync::Mutex<()>>,
    /// Exit proof survives taking/dropping the process handle.
    completion: Option<ProcessCompletion>,
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

const GRACEFUL_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const KILLED_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessExit {
    Pending,
    Terminated,
    Unconfirmed,
}

/// A private signal for one concrete process, never shared with its successor.
#[derive(Clone)]
struct ProcessCompletion {
    engine_id: usize,
    generation: u64,
    exit: tokio::sync::watch::Sender<ProcessExit>,
    root_exited: Arc<AtomicBool>,
    #[cfg(windows)]
    tree: Option<Arc<crate::windows_sidecar::WindowsJob>>,
}

impl ProcessCompletion {
    fn new(engine_id: usize, generation: u64) -> Self {
        let (exit, _) = tokio::sync::watch::channel(ProcessExit::Pending);
        Self {
            engine_id,
            generation,
            exit,
            root_exited: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            tree: None,
        }
    }

    fn for_child(engine_id: usize, generation: u64, _child: &CommandChild) -> Self {
        let completion = Self::new(engine_id, generation);
        #[cfg(windows)]
        let completion = Self {
            tree: Some(_child.job()),
            ..completion
        };
        completion
    }

    fn tree_empty(&self) -> Result<bool, String> {
        #[cfg(windows)]
        if let Some(tree) = &self.tree {
            return tree.is_empty();
        }
        Ok(true)
    }

    fn confirmed(&self) -> bool {
        let confirmed =
            self.root_exited.load(Ordering::Acquire) && self.tree_empty().unwrap_or(false);
        if confirmed {
            self.exit.send_replace(ProcessExit::Terminated);
        }
        confirmed
    }

    fn terminated(&self, engine_id: usize, generation: u64) {
        if self.engine_id == engine_id && self.generation == generation {
            self.root_exited.store(true, Ordering::Release);
            self.confirmed();
        }
    }

    fn stream_closed(&self) {
        // The bootloader can exit before its Python descendant. Keep the tree
        // pending until the OS reports zero active processes for this job.
        if self.root_exited.load(Ordering::Acquire) {
            return;
        }
        self.exit.send_if_modified(|exit| {
            if *exit == ProcessExit::Pending {
                *exit = ProcessExit::Unconfirmed;
                true
            } else {
                false
            }
        });
    }

    async fn wait(&self, timeout: std::time::Duration) -> Result<(), String> {
        let mut receiver = self.exit.subscribe();
        tokio::time::timeout(timeout, async {
            loop {
                if self.root_exited.load(Ordering::Acquire) && self.tree_empty()? {
                    self.exit.send_replace(ProcessExit::Terminated);
                    return Ok(());
                }
                let state = *receiver.borrow_and_update();
                match state {
                    ProcessExit::Terminated => return Ok(()),
                    ProcessExit::Unconfirmed => {
                        return Err("Sidecar event stream closed without confirmed termination."
                            .to_string())
                    }
                    ProcessExit::Pending => {}
                }
                tokio::select! {
                    changed = receiver.changed() => changed.map_err(|_| "Sidecar termination observer disappeared.".to_string())?,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {},
                }
            }
        })
        .await
        .map_err(|_| {
            format!(
                "Timed out awaiting sidecar {} generation {} termination.",
                self.engine_id, self.generation
            )
        })?
    }
}

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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub enum SafeDeleteOutcome {
    Safe(PathBuf),
    SafeJunction(PathBuf),
    Nonexistent,
    PolicyRefused(String),
}

#[cfg(test)]
fn is_reparse_point_or_symlink(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return true;
            }
        }
    }
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return true;
        }
    }
    false
}

#[cfg(test)]
fn check_ancestors_not_reparse(
    dir: &Path,
    library_root: &Path,
    canonical_root: &Path,
) -> Result<(), String> {
    let mut cur = dir.parent();
    while let Some(parent) = cur {
        if is_reparse_point_or_symlink(parent) {
            return Err(format!(
                "Ancestor directory '{}' is a junction or symlink: refusing to delete inside aliased tree {}",
                parent.display(),
                dir.display()
            ));
        }
        if parent == library_root || parent == canonical_root {
            break;
        }
        if let Ok(canon_p) = parent.canonicalize() {
            if canon_p == *canonical_root {
                break;
            }
        }
        cur = parent.parent();
    }
    Ok(())
}

#[cfg(test)]
pub fn safe_to_delete(dir: &Path, library_root: &Path) -> SafeDeleteOutcome {
    if !dir.exists() && !is_reparse_point_or_symlink(dir) {
        return SafeDeleteOutcome::Nonexistent;
    }

    let canonical_root = match library_root.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            return SafeDeleteOutcome::PolicyRefused(format!(
                "Failed to canonicalize library root {}: {e}",
                library_root.display()
            ))
        }
    };

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

    // Inspect every component along the path from library_root to dir.
    // If any parent component (e.g. library/snes) is a junction or symlink,
    // refuse deletion with SafeDeleteOutcome::PolicyRefused.
    if let Err(reason) = check_ancestors_not_reparse(dir, library_root, &canonical_root) {
        return SafeDeleteOutcome::PolicyRefused(reason);
    }

    if is_reparse_point_or_symlink(dir) {
        // A junction or symlink directory must NEVER be recursively wiped,
        // because remove_dir_all would delete the contents of the target directory!
        // Instead, the link itself should be removed via remove_dir.
        // We still MUST verify that the junction itself resides inside library_root
        // and is 2 levels deep (<category>/<game>) without being in a forbidden category.
        let parent = match dir.parent() {
            Some(p) => p,
            None => {
                return SafeDeleteOutcome::PolicyRefused(format!(
                    "Reparse point has no parent directory: {}",
                    dir.display()
                ))
            }
        };
        let canonical_parent = match parent.canonicalize() {
            Ok(cp) => cp,
            Err(e) => {
                return SafeDeleteOutcome::PolicyRefused(format!(
                    "Failed to canonicalize parent of reparse point {}: {e}",
                    dir.display()
                ))
            }
        };

        if canonical_parent.parent() != Some(&canonical_root) {
            return SafeDeleteOutcome::PolicyRefused(format!(
                "Reparse point {} parent is not directly inside library root {}",
                dir.display(),
                canonical_root.display()
            ));
        }

        let category = canonical_parent
            .file_name()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        if FORBIDDEN_TOP_CATEGORIES.contains(&category.as_str()) {
            return SafeDeleteOutcome::PolicyRefused(format!(
                "Refusing to delete junction inside reserved category '{}': {}",
                category,
                dir.display()
            ));
        }

        return SafeDeleteOutcome::SafeJunction(dir.to_path_buf());
    }

    let canonical_dir = match dir.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            return SafeDeleteOutcome::PolicyRefused(format!(
                "Failed to canonicalize delete path {}: {e}",
                dir.display()
            ))
        }
    };

    if canonical_dir == canonical_root
        || canonical_dir.parent() == Some(&canonical_root)
        || !canonical_dir.starts_with(&canonical_root)
    {
        return SafeDeleteOutcome::PolicyRefused(format!(
            "Refusing to delete root, parent category, or external directory: {}",
            canonical_dir.display()
        ));
    }

    let rel = match canonical_dir.strip_prefix(&canonical_root) {
        Ok(r) => r,
        Err(_) => {
            return SafeDeleteOutcome::PolicyRefused(format!(
                "Path {} is outside library root {}",
                canonical_dir.display(),
                canonical_root.display()
            ))
        }
    };

    let components: Vec<_> = rel
        .iter()
        .map(|c| c.to_string_lossy().to_string())
        .collect();
    if components.len() != 2 {
        return SafeDeleteOutcome::PolicyRefused(format!(
            "Delete path must be exactly 2 levels deep inside library root (<category>/<game>), got {}: {}",
            components.len(),
            canonical_dir.display()
        ));
    }

    let top_category = components[0].to_ascii_lowercase();
    if FORBIDDEN_TOP_CATEGORIES.contains(&top_category.as_str()) {
        return SafeDeleteOutcome::PolicyRefused(format!(
            "Refusing to delete inside reserved system/emulator category '{}': {}",
            top_category,
            canonical_dir.display()
        ));
    }

    SafeDeleteOutcome::Safe(canonical_dir)
}

enum SidecarLauncher {
    App(AppHandle),
    #[cfg(all(test, windows))]
    Executable(PathBuf),
}

pub struct LibtorrentEngine {
    launcher: SidecarLauncher,
    next_id: AtomicUsize,
    torrents: Arc<Mutex<HashMap<usize, TorrentEntry>>>,
    path_operations: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl LibtorrentEngine {
    fn save_identity(save_dir: &str) -> String {
        // Canonicalize the nearest existing ancestor so the key stays stable
        // when the sidecar creates a destination that did not exist at add.
        let input = PathBuf::from(save_dir);
        let absolute = if input.is_absolute() {
            input
        } else {
            std::env::current_dir().unwrap_or_default().join(input)
        };
        let mut path = PathBuf::new();
        for component in absolute.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    path.pop();
                }
                component => path.push(component.as_os_str()),
            }
        }
        let mut ancestor = path.as_path();
        let mut suffix = Vec::new();
        while ancestor.canonicalize().is_err() {
            let (Some(name), Some(parent)) = (ancestor.file_name(), ancestor.parent()) else {
                break;
            };
            suffix.push(name.to_owned());
            ancestor = parent;
        }
        if let Ok(canonical) = ancestor.canonicalize() {
            path = canonical;
            for component in suffix.into_iter().rev() {
                path.push(component);
            }
        }
        let identity = path.to_string_lossy().into_owned();
        #[cfg(windows)]
        let identity = identity.to_lowercase();
        identity
    }

    fn path_operation(&self, identity: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
        let mut operations = self
            .path_operations
            .lock()
            .map_err(|_| "Torrent path operation registry is poisoned.".to_string())?;
        Ok(Arc::clone(
            operations
                .entry(identity.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        ))
    }

    pub fn new(app: AppHandle) -> Self {
        Self {
            launcher: SidecarLauncher::App(app),
            next_id: AtomicUsize::new(1),
            torrents: Arc::new(Mutex::new(HashMap::new())),
            path_operations: Mutex::new(HashMap::new()),
        }
    }

    /// Native acceptance uses the production process/job path without a GUI.
    #[cfg(all(test, windows))]
    pub(crate) fn with_test_executable(executable: PathBuf) -> Self {
        Self {
            launcher: SidecarLauncher::Executable(executable),
            next_id: AtomicUsize::new(1),
            torrents: Arc::new(Mutex::new(HashMap::new())),
            path_operations: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(all(test, windows))]
    pub(crate) fn test_stderr(&self, id: usize) -> Vec<String> {
        self.lock()
            .ok()
            .and_then(|entries| entries.get(&id).map(|entry| entry.recent_stderr.clone()))
            .unwrap_or_default()
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
        #[cfg(all(test, windows))]
        if let SidecarLauncher::Executable(executable) = &self.launcher {
            let mut command = std::process::Command::new(executable);
            command.args([magnet, save_dir]);
            return crate::windows_sidecar::spawn(command);
        }
        #[cfg(not(all(test, windows)))]
        let SidecarLauncher::App(app) = &self.launcher;
        #[cfg(all(test, windows))]
        let app = match &self.launcher {
            SidecarLauncher::App(app) => app,
            #[cfg(all(test, windows))]
            SidecarLauncher::Executable(_) => {
                unreachable!("native executable launcher handled above")
            }
        };
        let command = app
            .shell()
            .sidecar(SIDECAR_NAME)
            .map_err(|error| format!("Failed to locate torrent sidecar: {error}"))?
            .args([magnet, save_dir]);
        #[cfg(windows)]
        let (rx, child) = crate::windows_sidecar::spawn(command.into())?;
        #[cfg(not(windows))]
        let (rx, child) = command
            .spawn()
            .map_err(|error| format!("Failed to start torrent sidecar: {error}"))?;

        Ok((rx, child))
    }

    fn start_sidecar_event_loop(
        &self,
        id: usize,
        generation: u64,
        completion: ProcessCompletion,
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
                                    if entry.generation == generation && !entry.user_stopped {
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
                        // Publish exit independently of entry/handle lifetime,
                        // including rejected spawns and obsolete generations.
                        completion.terminated(id, generation);
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
            completion.stream_closed();
            if *completion.exit.borrow() == ProcessExit::Unconfirmed {
                if let Ok(mut map) = torrents.lock() {
                    if let Some(entry) = map.get_mut(&id) {
                        if entry.generation == generation {
                            entry.stats.error = Some("Sidecar event stream closed without confirmed termination; files preserved.".to_string());
                        }
                    }
                }
            }
        });
    }
}

impl LibtorrentEngine {
    fn operation(&self, id: usize) -> Result<Option<Arc<tokio::sync::Mutex<()>>>, String> {
        Ok(self
            .lock()?
            .get(&id)
            .map(|entry| Arc::clone(&entry.operation)))
    }

    // Caller holds this entry's async operation lock. No std mutex spans await.
    async fn stop_locked(&self, id: usize, action: &str) -> Result<(), String> {
        let (completion, signaled) = {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.user_stopped = true;
            let command = format!("{{\"action\":\"{action}\"}}\n");
            let signaled = entry
                .child
                .as_mut()
                .is_some_and(|child| child.write(command.as_bytes()).is_ok());
            (entry.completion.clone(), signaled)
        };
        let Some(completion) = completion else {
            return Ok(());
        };
        if signaled && completion.wait(GRACEFUL_STOP_TIMEOUT).await.is_ok() {
            return Ok(());
        }
        // Taking or killing the handle is never evidence that writers stopped.
        let child = {
            let mut map = self.lock()?;
            map.get_mut(&id).and_then(|entry| entry.child.take())
        };
        #[cfg(windows)]
        let kill_error = if let Some(tree) = &completion.tree {
            // The bootloader handle may already be gone while Python is alive.
            // Kill the retained private job, never look up a recycled PID.
            let error = tree.terminate().err();
            drop(child);
            error
        } else {
            child.and_then(|child| child.kill().err())
        };
        #[cfg(not(windows))]
        let kill_error = child
            .and_then(|child| child.kill().err())
            .map(|error| error.to_string());
        let result = completion.wait(KILLED_STOP_TIMEOUT).await;
        if let Err(error) = &result {
            let error = format!("{error} Kill error: {kill_error:?}; files must be preserved.");
            if let Some(entry) = self.lock()?.get_mut(&id) {
                entry.stats.error = Some(error.clone());
            }
            return Err(error);
        }
        Ok(())
    }

    // Observer installation precedes every fallible post-spawn operation.
    async fn spawn_locked(&self, id: usize) -> Result<(), String> {
        let (magnet, save_dir, generation) = {
            let mut map = self.lock()?;
            let Some(identity) = map.get(&id).map(|entry| entry.save_identity.clone()) else {
                return Ok(());
            };
            if map.iter().any(|(other_id, entry)| {
                *other_id != id
                    && entry.save_identity == identity
                    && entry
                        .completion
                        .as_ref()
                        .map_or(!entry.user_stopped, |completion| !completion.confirmed())
            }) {
                return Err("Another torrent process may still write this destination.".to_string());
            }
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.generation += 1;
            entry.user_stopped = false;
            entry.recent_stderr.clear();
            entry.stats.error = None;
            (
                entry.magnet.clone(),
                entry.save_dir.clone(),
                entry.generation,
            )
        };
        let (rx, child) = match self.create_sidecar_process(&magnet, &save_dir) {
            Ok(pair) => pair,
            Err(error) => {
                if let Some(entry) = self.lock()?.get_mut(&id) {
                    entry.user_stopped = true;
                    entry.stats.error = Some(error.clone());
                }
                return Err(error);
            }
        };
        let completion = ProcessCompletion::for_child(id, generation, &child);
        self.start_sidecar_event_loop(id, generation, completion.clone(), rx);
        // Publish even a rejected process, so its exit can be awaited and retry
        // cannot evade an unresolved previous writer.
        let mut unpublished_child = Some(child);
        let publish_error = {
            match self.lock() {
                Ok(mut map) => {
                    if let Some(entry) = map.get_mut(&id) {
                        entry.completion = Some(completion.clone());
                        entry.child = unpublished_child.take();
                        None
                    } else {
                        Some("Torrent entry disappeared while spawning sidecar.".to_string())
                    }
                }
                Err(error) => Some(error),
            }
        };
        if let Some(error) = publish_error {
            // Even a poisoned map or rejected spawn keeps its event observer.
            if let Some(child) = unpublished_child {
                let _ = child.kill();
            }
            completion.wait(KILLED_STOP_TIMEOUT).await?;
            return Err(error);
        }
        Ok(())
    }
}

#[async_trait]
impl TorrentEngine for LibtorrentEngine {
    async fn add_magnet(&self, magnet_uri: String, save_dir: String) -> Result<usize, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let save_identity = Self::save_identity(&save_dir);
        let operation = self.path_operation(&save_identity)?;
        let _operation = operation.lock().await;
        {
            let mut map = self.lock()?;
            if map.values().any(|entry| {
                entry.save_identity == save_identity
                    && entry
                        .completion
                        .as_ref()
                        .map_or(!entry.user_stopped, |completion| !completion.confirmed())
            }) {
                return Err(
                "Another torrent process may still write this destination; stop is unconfirmed."
                    .to_string(),
            );
            }
            map.insert(
                id,
                TorrentEntry {
                    magnet: magnet_uri,
                    save_dir,
                    save_identity,
                    child: None,
                    generation: 0,
                    operation: Arc::clone(&operation),
                    completion: None,
                    stats: EngineTorrentStats::default(),
                    user_stopped: false,
                    recent_stderr: Vec::new(),
                },
            );
        }
        if let Err(error) = self.spawn_locked(id).await {
            // Retain unresolved generations. Losing their observer/entry would
            // allow a subsequent caller to mistake them for stopped writers.
            let removable = self.lock()?.get(&id).is_some_and(|entry| {
                entry
                    .completion
                    .as_ref()
                    .is_none_or(|completion| completion.confirmed())
            });
            if removable {
                self.lock()?.remove(&id);
            }
            return Err(error);
        }
        Ok(id)
    }

    async fn pause(&self, id: usize) -> Result<(), String> {
        let Some(operation) = self.operation(id)? else {
            return Ok(());
        };
        let _operation = operation.lock().await;
        self.stop_locked(id, "pause").await
    }

    async fn stop(&self, id: usize) -> Result<(), String> {
        let Some(operation) = self.operation(id)? else {
            return Ok(());
        };
        let _operation = operation.lock().await;
        self.stop_locked(id, "stop").await
    }

    async fn stop_for_path(&self, save_dir: &str) -> Result<(), String> {
        let identity = Self::save_identity(save_dir);
        let operation = self.path_operation(&identity)?;
        let _operation = operation.lock().await;
        let ids: Vec<_> = self
            .lock()?
            .iter()
            .filter(|(_, entry)| entry.save_identity == identity)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.stop_locked(id, "stop").await?;
        }
        Ok(())
    }

    async fn resume(&self, id: usize) -> Result<(), String> {
        let Some(operation) = self.operation(id)? else {
            return Ok(());
        };
        let _operation = operation.lock().await;
        {
            let map = self.lock()?;
            let Some(entry) = map.get(&id) else {
                return Ok(());
            };
            if !entry.user_stopped
                && entry
                    .completion
                    .as_ref()
                    .is_some_and(|completion| *completion.exit.borrow() == ProcessExit::Pending)
            {
                return Ok(());
            }
        }
        self.stop_locked(id, "stop").await?;
        self.spawn_locked(id).await
    }

    async fn delete(&self, id: usize, _delete_files: bool) -> Result<(), String> {
        let Some(operation) = self.operation(id)? else {
            return Ok(());
        };
        let _operation = operation.lock().await;
        self.stop_locked(id, "stop").await?;
        // Engine paths and archive-provided markers cannot establish ownership.
        // The manager reports preserved files or performs its proven cleanup.
        // In particular, false preserves existing payload AND fastresume data.
        self.lock()?.remove(&id);
        Ok(())
    }

    fn exists(&self, id: usize) -> bool {
        self.lock()
            .map(|map| map.contains_key(&id))
            .unwrap_or(false)
    }

    async fn approve_inventory(&self, id: usize) -> Result<(), String> {
        let operation = self.operation(id)?.ok_or("Torrent runtime missing.")?;
        let _operation = operation.lock().await;
        let mut entries = self.lock()?;
        let entry = entries.get_mut(&id).ok_or("Torrent runtime missing.")?;
        if entry.user_stopped || entry.stats.inventory.is_none() {
            return Err("Torrent inventory is not available for this runtime.".into());
        }
        entry
            .child
            .as_mut()
            .ok_or("Torrent process is no longer running.")?
            .write(b"{\"action\":\"inventory_approved\"}\n")
            .map_err(|error| format!("Could not approve torrent inventory: {error}"))
    }

    fn stats(&self, id: usize) -> Option<EngineTorrentStats> {
        self.lock().ok()?.get(&id).map(|entry| entry.stats.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_inventory_is_preserved_by_status_parser() {
        let stats = parse_progress_line(
            r#"{"progress":0,"state":"checking","inventory":["game/payload.bin",".hash.fastresume"]}"#,
        )
        .unwrap();
        assert_eq!(
            stats.inventory.unwrap(),
            vec!["game/payload.bin", ".hash.fastresume"]
        );
        assert!(!stats.finished);
        assert!(parse_progress_line(r#"{"progress":0}"#)
            .unwrap()
            .inventory
            .is_none());
    }

    #[test]
    fn destination_identity_survives_directory_creation() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("new").join("game");
        let before = LibtorrentEngine::save_identity(destination.to_str().unwrap());
        std::fs::create_dir_all(&destination).unwrap();
        assert_eq!(
            before,
            LibtorrentEngine::save_identity(destination.to_str().unwrap())
        );
    }

    #[tokio::test]
    async fn completion_requires_termination_for_the_same_process() {
        let completion = ProcessCompletion::new(7, 3);
        completion.terminated(7, 2);
        completion.terminated(8, 3);
        assert!(completion
            .wait(std::time::Duration::from_millis(1))
            .await
            .is_err());
        completion.terminated(7, 3);
        assert!(completion
            .wait(std::time::Duration::from_millis(1))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn closed_stream_without_termination_never_proves_exit() {
        let completion = ProcessCompletion::new(7, 3);
        completion.stream_closed();
        let error = completion
            .wait(std::time::Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(error.contains("without confirmed termination"));
    }

    #[tokio::test]
    async fn delayed_termination_releases_waiter_and_survives_stream_close() {
        let completion = ProcessCompletion::new(7, 3);
        let observer = completion.clone();
        let waiter =
            tokio::spawn(async move { completion.wait(std::time::Duration::from_secs(1)).await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        observer.terminated(7, 3);
        observer.stream_closed();
        assert!(waiter.await.unwrap().is_ok());
        assert!(observer
            .wait(std::time::Duration::from_millis(1))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn old_generation_cannot_release_new_generation_waiter() {
        let old = ProcessCompletion::new(7, 3);
        let new = ProcessCompletion::new(7, 4);
        old.terminated(7, 3);
        assert!(new.wait(std::time::Duration::from_millis(1)).await.is_err());
    }

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
            extract_btih(
                "magnet:?dn=Играть в игры&xt=urn:btih:0123456789abcdef0123456789abcdef01234567"
            ),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        // Cyrillic title after xt
        assert_eq!(
            extract_btih(
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Играть в игры"
            ),
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
        assert_eq!(
            extract_btih("magnet:?xt=urn:btih:0123456789abcdef&dn=short"),
            None
        );
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
        let canonical_game1 = match safe_to_delete(&game1_dir, &lib_root) {
            SafeDeleteOutcome::Safe(path) => path,
            other => panic!("Expected Safe outcome, got {:?}", other),
        };
        assert!(canonical_game1.ends_with("game1"));

        // Simulate deleting game1
        std::fs::remove_dir_all(&canonical_game1).unwrap();
        assert!(!game1_dir.exists());
        // Verify game2 is completely untouched!
        assert!(game2_dir.exists());
        assert_eq!(std::fs::read(game2_dir.join("rom.nes")).unwrap(), b"game2");

        // 2. Refuses to delete the entire library root itself
        assert!(matches!(
            safe_to_delete(&lib_root, &lib_root),
            SafeDeleteOutcome::PolicyRefused(_)
        ));

        // 3. Refuses to delete top-level category (e.g. library/downloads) which would wipe all games
        assert!(matches!(
            safe_to_delete(&downloads_category, &lib_root),
            SafeDeleteOutcome::PolicyRefused(_)
        ));

        // 4. Refuses to delete path outside library root
        assert!(matches!(
            safe_to_delete(&outside_dir, &lib_root),
            SafeDeleteOutcome::PolicyRefused(_)
        ));

        // 5. Nonexistent path is Nonexistent outcome
        assert_eq!(
            safe_to_delete(&downloads_category.join("already_deleted"), &lib_root),
            SafeDeleteOutcome::Nonexistent
        );

        // 6. Refuses to delete paths in reserved directories (shared-cache, emulators, etc.)
        let shared_cache_dir = lib_root.join("shared-cache").join("temporary");
        std::fs::create_dir_all(&shared_cache_dir).unwrap();
        assert!(matches!(
            safe_to_delete(&shared_cache_dir, &lib_root),
            SafeDeleteOutcome::PolicyRefused(_)
        ));

        let emulators_dir = lib_root.join("emulators").join("snes");
        std::fs::create_dir_all(&emulators_dir).unwrap();
        assert!(matches!(
            safe_to_delete(&emulators_dir, &lib_root),
            SafeDeleteOutcome::PolicyRefused(_)
        ));

        #[cfg(windows)]
        {
            let junction_link = downloads_category.join("game_link");
            let cmd_output = std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &junction_link.to_string_lossy(),
                    &game2_dir.to_string_lossy(),
                ])
                .output();
            if let Ok(output) = cmd_output {
                if output.status.success() {
                    assert_eq!(
                        safe_to_delete(&junction_link, &lib_root),
                        SafeDeleteOutcome::SafeJunction(junction_link.clone())
                    );
                    std::fs::remove_dir(&junction_link).unwrap();
                    assert!(!junction_link.exists());
                    assert!(game2_dir.exists());
                    assert_eq!(std::fs::read(game2_dir.join("rom.nes")).unwrap(), b"game2");
                }
            }
        }
    }

    #[test]
    #[cfg(windows)]
    fn safe_to_delete_refuses_when_parent_category_is_junction_and_protects_target() {
        let temp = tempfile::tempdir().unwrap();
        let lib_root = temp.path().join("library");
        let external_target = temp.path().join("external_games");
        let game_in_target = external_target.join("my_game");
        let secret_file = game_in_target.join("important_save.sav");

        std::fs::create_dir_all(&lib_root).unwrap();
        std::fs::create_dir_all(&game_in_target).unwrap();
        std::fs::write(&secret_file, b"precious_save_data").unwrap();

        // Create junction: lib_root/snes -> external_target
        let junction_category = lib_root.join("snes");
        let cmd_output = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &junction_category.to_string_lossy(),
                &external_target.to_string_lossy(),
            ])
            .output();

        if let Ok(output) = cmd_output {
            if output.status.success() {
                let delete_attempt = junction_category.join("my_game");
                let outcome = safe_to_delete(&delete_attempt, &lib_root);
                assert!(
                    matches!(outcome, SafeDeleteOutcome::PolicyRefused(_)),
                    "Expected PolicyRefused when parent category is junction, got {:?}",
                    outcome
                );

                // Verify the external target and secret file are completely untouched!
                assert!(secret_file.exists());
                assert_eq!(std::fs::read(&secret_file).unwrap(), b"precious_save_data");

                // Clean up junction
                let _ = std::fs::remove_dir(&junction_category);
            }
        }
    }
}
