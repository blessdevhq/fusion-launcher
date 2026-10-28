//! Host-selected offline repositories use the normal fetcher, CAS, workspace,
//! integrity guard and supervisor. The synthetic HTTPS identity is never fetched
//! over the network; only files beneath the explicitly selected directory exist.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;

use super::adapters::{ArtifactTransport, Cancellation, TransferLimits, TransferReceipt};
use super::canonical::QualifiedGameId;
use super::dto::{FusionGameState, GameOptions, InstallRequest, OperationProgress};
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::schema::SourceIndex;
use super::service::{FusionService, NoProgress, ServiceEnvironment};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};

fn failure(message: impl Into<String>) -> FusionError {
    FusionError::new(ErrorCode::InvalidPath, message)
}

#[cfg(test)]
#[path = "offline_tests.rs"]
mod tests;

pub struct OfflineTransport {
    root: PathBuf,
    base_url: String,
}

impl OfflineTransport {
    pub fn open(root: &Path, source_id: &str) -> Result<Self, FusionError> {
        fs_safety::validate_relative_path(source_id)?;
        if source_id.contains(['/', '\\']) {
            return Err(failure("Offline source ID must be one path component."));
        }
        let root = std::path::absolute(root).map_err(|e| failure(e.to_string()))?;
        fs_safety::guard(&root)?;
        Ok(Self {
            root,
            base_url: format!("https://offline.fusion.invalid/{source_id}/"),
        })
    }

    pub fn index_url(&self) -> String {
        format!("{}source-index.json", self.base_url)
    }

    fn resolve(&self, value: &str) -> Result<PathBuf, FusionError> {
        // Reject encoded aliases, query credentials, redirects and other origins.
        // Packer output has plain ASCII digest/ID paths; URL normalization must
        // never turn an untrusted path into authority over another directory.
        let relative = value.strip_prefix(&self.base_url).ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidUri, "Offline mode cannot fetch this URL.")
        })?;
        if relative.contains(['%', '?', '#', '\\']) {
            return Err(failure(
                "Encoded or decorated offline paths are not allowed.",
            ));
        }
        Ok(self.root.join(fs_safety::validate_relative_path(relative)?))
    }
}

#[async_trait]
impl ArtifactTransport for OfflineTransport {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        limits.validate()?;
        cancel.check()?;
        let source = self.resolve(url)?;
        let _source_guard = fs_safety::guard(source.parent().ok_or_else(|| failure("No parent"))?)?;
        let input = fs_safety::open_read(&source)?;
        let size = input.metadata().map_err(|e| failure(e.to_string()))?.len();
        if size > limits.max_bytes {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "Offline file exceeds transfer limit.",
            ));
        }
        let _destination_guard = fs_safety::guard(
            destination
                .parent()
                .ok_or_else(|| failure("No destination parent"))?,
        )?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let output = options
            .open(destination)
            .map_err(|e| failure(e.to_string()))?;
        let mut input = tokio::fs::File::from_std(input);
        // Finish each bounded write before yielding: dropping the fetch future
        // must not leave a background writer mutating its staging file.
        let mut output = output;
        let started = Instant::now();
        let mut total = 0;
        let mut buffer = [0u8; 64 * 1024];
        use tokio::io::AsyncReadExt;
        loop {
            cancel.check()?;
            if started.elapsed() > limits.timeout {
                return Err(FusionError::new(
                    ErrorCode::TransferLimitExceeded,
                    "Offline transfer timed out.",
                ));
            }
            let count = input
                .read(&mut buffer)
                .await
                .map_err(|e| failure(e.to_string()))?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > limits.max_bytes {
                return Err(FusionError::new(
                    ErrorCode::TransferLimitExceeded,
                    "Offline file grew beyond limit.",
                ));
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| failure(e.to_string()))?;
        }
        output.flush().map_err(|e| failure(e.to_string()))?;
        Ok(TransferReceipt {
            bytes: total,
            final_url: url.to_owned(),
        })
    }
}

pub struct OfflineLibrary {
    service: FusionService,
    storage: Arc<SqliteFusionStorage>,
    game: QualifiedGameId,
    // Retain both the directory guard and exclusive session lock for the entire
    // process lifetime, including descendants and save reconciliation.
    _session: File,
    _guard: fs_safety::DirectoryGuard,
}

/// Eden persists absolute directories and flips QSettings default flags on save.
/// Rebase only the data paths when a persistent profile enters a new workspace;
/// keep user input, graphics and audio settings byte-for-byte at line level.
fn relocate_eden_ini(text: &str, user_root: &Path) -> String {
    let prefix = user_root
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
        + "/";
    let mut section = "";
    let stale = text.lines().any(|line| {
        if line.starts_with('[') {
            section = line;
        }
        if section != "[Data%20Storage]" {
            return false;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        key.ends_with("_directory")
            && !value.is_empty()
            && !value.replace('\\', "/").to_lowercase().starts_with(&prefix)
    });
    if !stale {
        return text.to_owned();
    }
    section = "";
    text.lines()
        .map(|line| {
            if line.starts_with('[') {
                section = line;
            }
            if section == "[Data%20Storage]" {
                if let Some((key, _)) = line.split_once('=') {
                    if key.ends_with("_directory\\default") {
                        return format!("{key}=true");
                    }
                    if key.ends_with("_directory") {
                        return format!("{key}=");
                    }
                }
            }
            line.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn register_eden_game_dir(text: &str, game_dir: &Path) -> String {
    let directory = game_dir.to_string_lossy().replace('\\', "/");
    if text
        .lines()
        .any(|l| l == format!("Paths\\gamedirs\\1\\path={directory}"))
        && text.lines().any(|l| l == "Paths\\gamedirs\\size=1")
    {
        return text.to_owned();
    }
    let entries = format!("Paths\\gamedirs\\size=1\nPaths\\gamedirs\\1\\path={directory}\nPaths\\gamedirs\\1\\deep_scan=false\nPaths\\gamedirs\\1\\expanded=true\nPaths\\gamedirs\\1\\deep_scan\\default=true\nPaths\\gamedirs\\1\\expanded\\default=true\n");
    let mut in_ui = false;
    let mut found = false;
    let mut output = String::new();
    for line in text.lines() {
        if line.starts_with('[') {
            if in_ui {
                output.push_str(&entries);
            }
            in_ui = line == "[UI]";
            found |= in_ui;
        }
        if in_ui && line.starts_with("Paths\\gamedirs\\") {
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    if in_ui {
        output.push_str(&entries);
    }
    if !found {
        output.push_str("\n[UI]\n");
        output.push_str(&entries);
    }
    output
}

impl OfflineLibrary {
    pub async fn open(source: &Path, library: &Path, game_id: &str) -> Result<Self, FusionError> {
        let source = std::path::absolute(source).map_err(|e| failure(e.to_string()))?;
        let library = std::path::absolute(library).map_err(|e| failure(e.to_string()))?;
        let mut text = String::new();
        fs_safety::open_read(&source.join("source-index.json"))?
            .take(2 * 1024 * 1024)
            .read_to_string(&mut text)
            .map_err(|e| failure(e.to_string()))?;
        let index: SourceIndex = serde_json::from_str(&text).map_err(|e| failure(e.to_string()))?;
        if !index.games.iter().any(|g| g.id == game_id) {
            return Err(failure(
                "Requested game is not present in this offline source.",
            ));
        }
        let transport = Arc::new(OfflineTransport::open(&source, &index.source_info.id)?);
        let index_url = transport.index_url();
        let control = library.join(".fusion-source/offline-host");
        let guard = fs_safety::ensure_directory(&control)?;
        let lock_path = control.join("session.lock");
        if let Ok(meta) = std::fs::symlink_metadata(&lock_path) {
            fs_safety::check_metadata(&meta, &lock_path)?;
        }
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            opts.share_mode(3).custom_flags(0x00200000);
        }
        let session = opts.open(&lock_path).map_err(|e| failure(e.to_string()))?;
        fs_safety::check_metadata(
            &session.metadata().map_err(|e| failure(e.to_string()))?,
            &lock_path,
        )?;
        fs2::FileExt::try_lock_exclusive(&session).map_err(|_| {
            FusionError::new(
                ErrorCode::OperationInProgress,
                "Another offline Fusion session owns this library.",
            )
        })?;
        let database = control.join("library.sqlite");
        if let Ok(meta) = std::fs::symlink_metadata(&database) {
            fs_safety::check_metadata(&meta, &database)?;
        }
        let store = RepositoryStore::open(&database).map_err(failure)?;
        let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(store))));
        storage.interrupt_operations()?;
        let appdata = control.join("appdata");
        let local_appdata = control.join("localappdata");
        fs_safety::ensure_directory(&appdata)?;
        fs_safety::ensure_directory(&local_appdata)?;
        let service = FusionService::open(
            &library,
            storage.clone(),
            transport,
            Arc::new(NoProgress),
            ServiceEnvironment {
                appdata,
                local_appdata,
                display_width: 1920,
                display_height: 1080,
                refresh_rate: 60,
            },
        )?;
        // Normal source signature/identity and raw-byte manifest checks still run.
        service.source_add(&index_url).await?;
        Ok(Self {
            service,
            storage,
            game: QualifiedGameId {
                source_id: index.source_info.id,
                game_id: game_id.into(),
            },
            _session: session,
            _guard: guard,
        })
    }

    pub async fn install(&self) -> Result<OperationProgress, FusionError> {
        self.install_at(None).await
    }
    pub async fn install_at(
        &self,
        external_root: Option<String>,
    ) -> Result<OperationProgress, FusionError> {
        self.service
            .game_install(
                &self.game,
                InstallRequest {
                    external_workspace_root: external_root,
                    ..InstallRequest::default()
                },
            )
            .await
    }
    pub async fn state(&self) -> Result<FusionGameState, FusionError> {
        self.service.game_get_state(&self.game).await
    }
    /// Stage a validated selection. The active snapshot changes only on update.
    pub async fn set_options(&self, options: GameOptions) -> Result<FusionGameState, FusionError> {
        self.service.game_set_options(&self.game, options).await
    }
    /// Apply the selected source revision/options through the normal transaction.
    /// The service retains the installed workspace root and persistence bindings.
    pub async fn update(&self) -> Result<OperationProgress, FusionError> {
        self.service
            .game_update(&self.game, InstallRequest::default())
            .await
    }
    pub async fn trust(&self, path: &str, digest: &str) -> Result<(), FusionError> {
        self.service
            .game_trust_executable(&self.game, path, digest)
            .await
    }
    pub async fn trust_many(&self, files: Vec<(String, String)>) -> Result<(), FusionError> {
        self.service.game_trust_executables(&self.game, files).await
    }
    pub async fn launch(&self) -> Result<OperationProgress, FusionError> {
        // Wait for the real supervisor and all descendants; do not detach.
        self.service.game_launch(&self.game).await
    }
    pub async fn launch_eden(&self) -> Result<OperationProgress, FusionError> {
        self.prepare_eden_profile().await?;
        self.launch().await
    }
    pub async fn prepare_eden_profile(&self) -> Result<(), FusionError> {
        let root = self.service.library_root().to_owned();
        let game = self.game.clone();
        use super::service::FusionStorage;
        let external = self
            .storage
            .get_installation(&game)?
            .and_then(|r| r.external_workspace_root);
        tokio::task::spawn_blocking(move || {
            let mut assembler = super::workspace::WorkspaceAssembler::open(&root, &game)?;
            if let Some(external) = external {
                assembler = assembler.with_external_workspace_root(Path::new(&external))?;
            }
            let lease = assembler
                .active()?
                .ok_or_else(|| failure("No installed Eden workspace"))?;
            prepare_eden_workspace(&lease)?;
            Ok(())
        })
        .await
        .map_err(|_| failure("Eden profile preparation failed"))??;
        Ok(())
    }
    pub async fn rollback(&self) -> Result<OperationProgress, FusionError> {
        self.service.game_rollback(&self.game).await
    }
}

/// Host-only preparation of an already leased, isolated portable profile.
pub(crate) fn prepare_eden_workspace(
    lease: &super::workspace::AssembledWorkspace,
) -> Result<(), FusionError> {
    if lease
        .snapshot
        .manifest
        .runtime
        .as_ref()
        .is_none_or(|r| r.executable != "runtime/eden.exe")
    {
        return Err(failure("Eden profile relocation requires runtime/eden.exe"));
    }
    if !lease
        .snapshot
        .manifest
        .persistence
        .saves_paths
        .iter()
        .any(|p| p == "workspace/runtime/user")
    {
        return Err(failure(
            "Eden profile relocation requires persistent runtime/user",
        ));
    }
    let user = lease.snapshot.workspace_root.join("runtime/user");
    let binding = lease
        .snapshot
        .persistence
        .bindings
        .iter()
        .find(|b| b.workspace_relative.as_deref() == Some("runtime/user"))
        .ok_or_else(|| failure("Missing managed Eden persistence binding"))?;
    // Windows PersistenceManager owns a junction here. Use its verified
    // plain data root; do not weaken the no-reparse filesystem helpers.
    let data = &binding.persistent_path;
    if fs_safety::list_plain_files(data)?
        .iter()
        .any(|(relative, _)| user.join(relative).to_string_lossy().encode_utf16().count() >= 250)
    {
        return Err(failure("Eden user paths exceed the safe Windows path budget. Install with a shorter --workspace-root."));
    }
    #[cfg(windows)]
    let config = data.join("config/qt-config.ini");
    #[cfg(not(windows))]
    let config = user.join("config/qt-config.ini");
    let mut text = String::new();
    fs_safety::open_read(&config)?
        .take(4 * 1024 * 1024)
        .read_to_string(&mut text)
        .map_err(|e| failure(e.to_string()))?;
    let relocated = register_eden_game_dir(
        &relocate_eden_ini(&text, &user),
        &lease.snapshot.workspace_root.join("game"),
    );
    if relocated != text {
        fs_safety::atomic_write(&config, relocated.as_bytes())?;
        lease.reconcile_saves()?;
    }
    Ok(())
}
