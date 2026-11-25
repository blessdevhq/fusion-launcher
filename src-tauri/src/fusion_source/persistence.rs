//! Per-game saves live outside replaceable workspace versions. Initial imports
//! preserve the user's originals; binary rollback never restores save data.
//! The caller must stop all game writers before taking a save snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::adapters::Cancellation;
use super::canonical::QualifiedGameId;
use super::dto::{SaveSnapshotInventory, SaveSnapshotSummary};
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::schema::PersistenceSpec;

#[derive(Debug, Clone)]
pub struct PersistenceRoots {
    pub appdata: PathBuf,
    pub local_appdata: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavePathKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistenceBinding {
    pub declared_path: String,
    pub persistent_path: PathBuf,
    pub workspace_relative: Option<String>,
    pub kind: SavePathKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistenceBindings {
    pub bindings: Vec<PersistenceBinding>,
    pub env_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveSnapshot {
    pub id: String,
    pub root: PathBuf,
    pub files: BTreeMap<String, u64>,
    pub total_bytes: u64,
}

#[derive(Debug)]
pub struct PersistenceManager {
    root: PathBuf,
}

impl PersistenceManager {
    pub fn open(library_root: &Path, game: &QualifiedGameId) -> Result<Self, FusionError> {
        let root = library_root.join(persistence_path(game)?);
        let _library = fs_safety::guard(library_root)?;
        let _root = fs_safety::ensure_directory(&root)?;
        let _data = fs_safety::ensure_directory(&root.join("data"))?;
        let _snapshots = fs_safety::ensure_directory(&root.join("snapshots"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Missing paths are directories. Existing file saves retain their file kind.
    /// Only declared paths are imported; global APPDATA trees are never redirected
    /// with a junction or modified in place.
    pub fn prepare(
        &self,
        workspace: &Path,
        spec: &PersistenceSpec,
        roots: &PersistenceRoots,
    ) -> Result<PersistenceBindings, FusionError> {
        let _lock = self.lock()?;
        let _workspace = fs_safety::guard(workspace)?;
        let paths = validated_paths(spec)?;
        let mut result = PersistenceBindings::default();
        for path in paths {
            let persistent = self
                .root
                .join("data")
                .join(path.namespace)
                .join(&path.relative);
            let original_root = match path.namespace {
                "workspace" => workspace,
                "appdata" => &roots.appdata,
                "localappdata" => &roots.local_appdata,
                _ => unreachable!("validated save namespace"),
            };
            let original = original_root.join(&path.relative);
            let _original_root = fs_safety::guard(original_root)?;
            let kind = match fs::symlink_metadata(&persistent) {
                Ok(metadata) => {
                    check_plain(&persistent, &metadata)?;
                    if metadata.is_dir() {
                        SavePathKind::Directory
                    } else {
                        SavePathKind::File
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let _parent = fs_safety::ensure_directory(
                        persistent.parent().expect("persistent save parent"),
                    )?;
                    match fs::symlink_metadata(&original) {
                        Ok(metadata) => {
                            check_plain(&original, &metadata)?;
                            if metadata.is_dir() {
                                // Copy to a private temporary slot, then publish only a complete import.
                                let imports = self.root.join("imports");
                                let _imports = fs_safety::ensure_directory(&imports)?;
                                let staging = imports.join(new_id());
                                copy_tree(&original, &staging, &mut BTreeMap::new(), "")?;
                                fs::rename(&staging, &persistent).map_err(|error| {
                                    io_error("publish save import", &persistent, error)
                                })?;
                                SavePathKind::Directory
                            } else {
                                fs_safety::copy_file(&original, &persistent)?;
                                SavePathKind::File
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            // Check any existing ancestors even when the final save path is absent.
                            check_existing_ancestors(&original, original_root)?;
                            let _slot = fs_safety::ensure_directory(&persistent)?;
                            SavePathKind::Directory
                        }
                        Err(error) => {
                            return Err(io_error("inspect original save", &original, error))
                        }
                    }
                }
                Err(error) => return Err(io_error("inspect persistent save", &persistent, error)),
            };
            result.bindings.push(PersistenceBinding {
                declared_path: path.declared.to_owned(),
                persistent_path: persistent,
                workspace_relative: (path.namespace == "workspace").then(|| path.relative.clone()),
                kind,
            });
            if path.namespace != "workspace" {
                let variable = if path.namespace == "appdata" {
                    "APPDATA"
                } else {
                    "LOCALAPPDATA"
                };
                let redirected = self.root.join("data").join(path.namespace);
                let _redirected = fs_safety::ensure_directory(&redirected)?;
                result.env_overrides.insert(
                    variable.into(),
                    redirected
                        .to_str()
                        .ok_or_else(|| invalid("Save paths must be valid Unicode."))?
                        .to_owned(),
                );
            }
        }
        Ok(result)
    }

    /// Bind only an owned, inactive workspace. Directory links are explicit owned
    /// junctions on Windows; cleanup must unlink them without following targets.
    pub fn bind_workspace(
        &self,
        workspace: &Path,
        bindings: &PersistenceBindings,
    ) -> Result<(), FusionError> {
        let _lock = self.lock()?;
        let _workspace = fs_safety::guard(workspace)?;
        for binding in &bindings.bindings {
            let parsed = parse_path(&binding.declared_path)?;
            let expected = self
                .root
                .join("data")
                .join(parsed.namespace)
                .join(&parsed.relative);
            if binding.persistent_path != expected
                || binding.workspace_relative
                    != (parsed.namespace == "workspace").then(|| parsed.relative.clone())
            {
                return Err(invalid("Save binding does not belong to this game."));
            }
            let Some(relative) = &binding.workspace_relative else {
                continue;
            };
            let target = workspace.join(fs_safety::validate_relative_path(relative)?);
            let _parent =
                fs_safety::ensure_directory(target.parent().expect("workspace save parent"))?;
            let persistent_metadata = fs::symlink_metadata(&binding.persistent_path)
                .map_err(|error| io_error("inspect save slot", &binding.persistent_path, error))?;
            check_plain(&binding.persistent_path, &persistent_metadata)?;
            if (binding.kind == SavePathKind::Directory) != persistent_metadata.is_dir() {
                return Err(invalid("Save slot kind changed."));
            }
            if let Ok(metadata) = fs::symlink_metadata(&target) {
                if is_reparse(&metadata) {
                    let existing = fs::read_link(&target)
                        .map_err(|error| io_error("read save junction", &target, error))?;
                    let expected = fs::canonicalize(&binding.persistent_path).map_err(|error| {
                        io_error("resolve save slot", &binding.persistent_path, error)
                    })?;
                    if existing != expected
                        && fs::canonicalize(&existing).ok().as_ref() != Some(&expected)
                    {
                        return Err(invalid("Refusing to replace an unowned save link."));
                    }
                    continue;
                }
                if binding.kind == SavePathKind::File
                    && metadata.is_file()
                    && fs_safety::file_identity(&target)?
                        == fs_safety::file_identity(&binding.persistent_path)?
                {
                    continue;
                }
                if metadata.is_dir() {
                    fs_safety::safe_remove_tree(&target)?;
                } else {
                    fs_safety::unlink_file(&target)?;
                }
            }
            match binding.kind {
                SavePathKind::Directory => {
                    create_directory_link(&binding.persistent_path, &target)?
                }
                SavePathKind::File => {
                    if fs_safety::same_volume(
                        &binding.persistent_path,
                        target.parent().expect("save link parent"),
                    )? {
                        fs::hard_link(&binding.persistent_path, &target).map_err(|error| {
                            io_error("link persistent file save", &target, error)
                        })?;
                    } else {
                        create_file_link(&binding.persistent_path, &target)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Remove only bindings to this manager's exact save slots before the caller
    /// cleans an inactive workspace. This never recursively deletes save data.
    pub fn unbind_workspace(
        &self,
        workspace: &Path,
        bindings: &PersistenceBindings,
    ) -> Result<(), FusionError> {
        let _lock = self.lock()?;
        let _workspace = fs_safety::guard(workspace)?;
        for binding in &bindings.bindings {
            let parsed = parse_path(&binding.declared_path)?;
            let expected = self
                .root
                .join("data")
                .join(parsed.namespace)
                .join(&parsed.relative);
            if binding.persistent_path != expected
                || binding.workspace_relative
                    != (parsed.namespace == "workspace").then(|| parsed.relative.clone())
            {
                return Err(invalid("Save binding does not belong to this game."));
            }
            let Some(relative) = &binding.workspace_relative else {
                continue;
            };
            let target = workspace.join(fs_safety::validate_relative_path(relative)?);
            let _parent = fs_safety::guard(target.parent().expect("workspace save parent"))?;
            let metadata = match fs::symlink_metadata(&target) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error("inspect save binding", &target, error)),
            };
            if is_reparse(&metadata) {
                let actual = fs::read_link(&target)
                    .map_err(|error| io_error("read save link", &target, error))?;
                let expected = fs::canonicalize(&binding.persistent_path).map_err(|error| {
                    io_error("resolve save slot", &binding.persistent_path, error)
                })?;
                if actual != expected && fs::canonicalize(&actual).ok().as_ref() != Some(&expected)
                {
                    return Err(invalid("Refusing to remove an unowned save link."));
                }
                if binding.kind == SavePathKind::Directory {
                    unlink_workspace_save_link(&target)?;
                } else {
                    fs::remove_file(&target)
                        .map_err(|error| io_error("unlink file save", &target, error))?;
                }
            } else if binding.kind == SavePathKind::File
                && metadata.is_file()
                && fs_safety::file_identity(&target)?
                    == fs_safety::file_identity(&binding.persistent_path)?
            {
                fs_safety::unlink_file(&target)?;
            } else {
                return Err(invalid(
                    "Refusing to remove a path that is not the recorded save binding.",
                ));
            }
        }
        Ok(())
    }

    /// After stopping writers, capture file saves which a game replaced by
    /// temp-file + rename. Such a replacement breaks the original hardlink.
    /// Reconcile the CURRENT workspace before backup or binary rollback, never
    /// an old rollback target whose bytes may be stale.
    pub fn reconcile_workspace(
        &self,
        workspace: &Path,
        bindings: &PersistenceBindings,
    ) -> Result<(), FusionError> {
        let _lock = self.lock()?;
        let _workspace = fs_safety::guard(workspace)?;
        for binding in &bindings.bindings {
            let parsed = parse_path(&binding.declared_path)?;
            let expected = self
                .root
                .join("data")
                .join(parsed.namespace)
                .join(&parsed.relative);
            if binding.persistent_path != expected
                || binding.workspace_relative
                    != (parsed.namespace == "workspace").then(|| parsed.relative.clone())
            {
                return Err(invalid("Save binding does not belong to this game."));
            }
            let Some(relative) = &binding.workspace_relative else {
                continue;
            };
            if binding.kind != SavePathKind::File {
                continue;
            }
            let target = workspace.join(fs_safety::validate_relative_path(relative)?);
            let _parent = fs_safety::guard(target.parent().expect("workspace save parent"))?;
            let persistent_metadata =
                fs::symlink_metadata(&binding.persistent_path).map_err(|error| {
                    io_error(
                        "inspect persistent file save",
                        &binding.persistent_path,
                        error,
                    )
                })?;
            check_plain(&binding.persistent_path, &persistent_metadata)?;
            if !persistent_metadata.is_file() {
                return Err(invalid("File save slot changed kind."));
            }
            let metadata = fs::symlink_metadata(&target)
                .map_err(|error| io_error("inspect current workspace file save", &target, error))?;
            if is_reparse(&metadata) {
                let actual = fs::read_link(&target)
                    .map_err(|error| io_error("read current file save link", &target, error))?;
                let expected = fs::canonicalize(&binding.persistent_path).map_err(|error| {
                    io_error("resolve file save slot", &binding.persistent_path, error)
                })?;
                if actual != expected && fs::canonicalize(&actual).ok().as_ref() != Some(&expected)
                {
                    return Err(invalid(
                        "Current file save link points outside its recorded slot.",
                    ));
                }
                continue;
            }
            check_plain(&target, &metadata)?;
            if !metadata.is_file() {
                return Err(invalid("Current file save changed kind."));
            }
            if fs_safety::file_identity(&target)?
                != fs_safety::file_identity(&binding.persistent_path)?
            {
                fs_safety::atomic_copy_file(&target, &binding.persistent_path)?;
            }
        }
        Ok(())
    }

    /// Complete this backup before activating a replacement binary workspace.
    /// An interrupted copy remains under an incomplete name and is never returned.
    pub fn snapshot_before_update(&self) -> Result<SaveSnapshot, FusionError> {
        self.create_snapshot(None)
    }

    pub(super) fn snapshot_manually(
        &self,
        workspace: &Path,
        bindings: &PersistenceBindings,
        cancel: &Cancellation,
    ) -> Result<(), FusionError> {
        self.create_snapshot(Some((workspace, bindings, cancel)))
            .map(drop)
    }

    fn create_snapshot(
        &self,
        manual: Option<(&Path, &PersistenceBindings, &Cancellation)>,
    ) -> Result<SaveSnapshot, FusionError> {
        let _lock = self.lock()?;
        let id = new_id();
        let staging = self.root.join("snapshots").join(format!("{id}.incomplete"));
        let root = self.root.join("snapshots").join(&id);
        let mut files = BTreeMap::new();
        if let Some((workspace, bindings, cancel)) = manual {
            cancel.check()?;
            self.copy_active_saves(workspace, bindings, &staging, &mut files, cancel)?;
        } else {
            copy_tree(&self.root.join("data"), &staging, &mut files, "")?;
        }
        let total_bytes = files
            .values()
            .try_fold(0u64, |total, size| total.checked_add(*size))
            .ok_or_else(|| invalid("Save backup size overflow."))?;
        let snapshot = SaveSnapshot {
            id,
            root,
            files,
            total_bytes,
        };
        let metadata = serde_json::to_vec(&snapshot)
            .map_err(|error| fs_safety::io_error("Serialize save metadata", &staging, error))?;
        if manual.is_some() {
            if snapshot.files.is_empty() {
                return Err(FusionError::new(
                    ErrorCode::PersistenceConflict,
                    "No save files have been written yet.",
                )
                .with_detail("reason", "no_save_files"));
            }
            if snapshot.files.len() > MAX_SAVE_METADATA_FILES
                || snapshot.total_bytes > MAX_EXACT_IPC_BYTES
                || metadata.len() as u64 > MAX_SAVE_METADATA_BYTES
            {
                return Err(invalid(
                    "Save backup exceeds the supported inventory limits.",
                ));
            }
        }
        fs_safety::atomic_write(&staging.join(".snapshot.json"), &metadata)?;
        let _parent = fs_safety::guard(&self.root.join("snapshots"))?;
        if let Some((_, _, cancel)) = manual {
            cancel.check()?;
        }
        fs::rename(&staging, &snapshot.root)
            .map_err(|error| io_error("publish save snapshot", &snapshot.root, error))?;
        Ok(snapshot)
    }

    /// Read the current save view without reconciling or changing live slots.
    fn copy_active_saves(
        &self,
        workspace: &Path,
        bindings: &PersistenceBindings,
        staging: &Path,
        files: &mut BTreeMap<String, u64>,
        cancel: &Cancellation,
    ) -> Result<(), FusionError> {
        let _workspace = fs_safety::guard(workspace)?;
        for binding in &bindings.bindings {
            cancel.check()?;
            let parsed = parse_path(&binding.declared_path)?;
            let relative = format!("{}/{}", parsed.namespace, parsed.relative);
            let persistent = self.root.join("data").join(&relative);
            if persistent != binding.persistent_path
                || binding.workspace_relative
                    != (parsed.namespace == "workspace").then_some(parsed.relative)
            {
                return Err(invalid("Save binding does not belong to this game."));
            }
            let _persistent_parent = fs_safety::guard(persistent.parent().expect("save parent"))?;
            let metadata = fs::symlink_metadata(&persistent)
                .map_err(|error| io_error("inspect save slot", &persistent, error))?;
            check_plain(&persistent, &metadata)?;
            let directory = binding.kind == SavePathKind::Directory;
            if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
                return Err(invalid("Save slot kind changed."));
            }
            let target = binding
                .workspace_relative
                .as_deref()
                .map(|path| workspace.join(path));
            let _target_parent = target
                .as_deref()
                .and_then(Path::parent)
                .map(fs_safety::guard)
                .transpose()?;
            let mut source = persistent.as_path();
            if let Some(target) = &target {
                let metadata = fs::symlink_metadata(target)
                    .map_err(|error| io_error("inspect workspace save", target, error))?;
                if is_reparse(&metadata) {
                    let actual = fs::read_link(target)
                        .map_err(|error| io_error("read workspace save link", target, error))?;
                    let expected = fs::canonicalize(&persistent)
                        .map_err(|error| io_error("resolve save slot", &persistent, error))?;
                    let actual_plain = fs::symlink_metadata(&actual)
                        .ok()
                        .is_some_and(|meta| !is_reparse(&meta));
                    if actual != expected
                        && actual != persistent
                        && !(actual_plain
                            && fs::canonicalize(&actual).ok().as_ref() == Some(&expected))
                    {
                        return Err(invalid(
                            "Workspace save link points outside its recorded slot.",
                        ));
                    }
                } else if !directory && metadata.is_file() {
                    source = target;
                } else {
                    return Err(invalid("Workspace save directory or file binding changed."));
                }
            }
            let destination = staging.join(&relative);
            if directory {
                copy_tree(source, &destination, files, &relative)?;
            } else {
                fs_safety::copy_file(source, &destination)?;
                let copied = fs::metadata(&destination)
                    .map_err(|error| io_error("inspect staged save", &destination, error))?;
                files.insert(relative, copied.len());
            }
        }
        Ok(())
    }

    fn lock(&self) -> Result<PersistenceLock, FusionError> {
        let _root = fs_safety::guard(&self.root)?;
        let path = self.root.join(".lock");
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            check_plain(&path, &metadata)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| io_error("open save lock", &path, error))?;
        file.lock_exclusive()
            .map_err(|error| io_error("lock saves", &path, error))?;
        Ok(PersistenceLock { _file: file })
    }
}

const MAX_LISTED_SAVE_SNAPSHOTS: usize = 100;
const MAX_SCANNED_SAVE_ENTRIES: usize = 4096;
const MAX_READ_SAVE_METADATA: usize = 256;
const MAX_SAVE_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_SAVE_METADATA_FILES: usize = 10_000;
const MAX_EXACT_IPC_BYTES: u64 = (1 << 53) - 1;

fn persistence_path(game: &QualifiedGameId) -> Result<PathBuf, FusionError> {
    Ok(Path::new(".fusion-source")
        .join("persistence")
        .join(safe_id(&game.source_id)?)
        .join(safe_id(&game.game_id)?))
}

/// List only published metadata. Never open the persistence manager, lock save
/// writers, or resolve paths from a metadata document.
pub(super) fn list_save_snapshots(
    library_root: &Path,
    game: &QualifiedGameId,
) -> Result<SaveSnapshotInventory, FusionError> {
    let unavailable = |code| FusionError::new(code, "Cannot read the save backup directory.");
    let relative = persistence_path(game)
        .map_err(|error| unavailable(error.code))?
        .join("snapshots");
    let mut inventory = SaveSnapshotInventory::default();
    let mut guard = fs_safety::guard(library_root).map_err(|error| unavailable(error.code))?;
    let mut snapshots = library_root.to_owned();
    for part in relative.components() {
        snapshots.push(part);
        // Check only a direct child of the guarded directory. A junction with
        // no snapshots beneath it is unsafe, not a missing inventory.
        match fs::symlink_metadata(&snapshots) {
            Ok(metadata) => {
                fs_safety::check_metadata(&metadata, &snapshots)
                    .map_err(|error| unavailable(error.code))?;
                if !metadata.is_dir() {
                    return Err(unavailable(ErrorCode::InvalidPath));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(inventory),
            Err(_) => return Err(unavailable(ErrorCode::Io)),
        }
        // Each guard also protects all ancestors; keep the deepest one live.
        guard = fs_safety::guard(&snapshots).map_err(|error| unavailable(error.code))?;
    }
    let _snapshots_guard = guard;

    let mut entries = fs::read_dir(&snapshots).map_err(|_| unavailable(ErrorCode::Io))?;
    let mut ids = Vec::new();
    for entry in entries.by_ref().take(MAX_SCANNED_SAVE_ENTRIES) {
        let entry = entry.map_err(|_| unavailable(ErrorCode::Io))?;
        if let Ok(id) = entry.file_name().into_string() {
            if is_save_snapshot_id(&id) {
                ids.push(id);
            }
        }
    }
    // At the directory bound, newest-first applies to the entries encountered,
    // not to entries outside the scan. Report that the inventory is incomplete.
    inventory.has_more = entries.next().is_some() || ids.len() > MAX_READ_SAVE_METADATA;
    ids.sort_unstable_by(|left, right| right.cmp(left));
    for id in ids.into_iter().take(MAX_READ_SAVE_METADATA) {
        match read_save_snapshot_summary(&snapshots.join(&id), &id) {
            Some(summary) if inventory.snapshots.len() < MAX_LISTED_SAVE_SNAPSHOTS => {
                inventory.snapshots.push(summary);
            }
            Some(_) => inventory.has_more = true,
            None => inventory.skipped_count += 1,
        }
    }
    Ok(inventory)
}

fn is_save_snapshot_id(id: &str) -> bool {
    // new_id(): 32 lowercase hexadecimal epoch nanoseconds, then 16 sequence digits.
    id.len() == 49
        && id.bytes().enumerate().all(|(index, byte)| {
            if index == 32 {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
            }
        })
}

fn read_save_snapshot_summary(root: &Path, id: &str) -> Option<SaveSnapshotSummary> {
    let nanos = u128::from_str_radix(id.split_once('-')?.0, 16).ok()?;
    let seconds = i64::try_from(nanos / 1_000_000_000).ok()?;
    if seconds > 253_402_300_799 {
        return None; // RFC3339 requires a four-digit year.
    }
    let created_at = DateTime::from_timestamp(seconds, (nanos % 1_000_000_000) as u32)?;
    let _entry_guard = fs_safety::guard(root).ok()?;
    let file = fs_safety::open_read(&root.join(".snapshot.json")).ok()?;
    let length = file.metadata().ok()?.len();
    if length > MAX_SAVE_METADATA_BYTES || fs_safety::link_count(&file).ok()? != 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_SAVE_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 != length {
        return None;
    }
    let metadata: SaveSnapshot = serde_json::from_slice(&bytes).ok()?;
    if metadata.id != id
        || metadata.files.len() > MAX_SAVE_METADATA_FILES
        || metadata.total_bytes > MAX_EXACT_IPC_BYTES
        || !save_snapshot_root_matches(root, &metadata.root)
    {
        return None;
    }
    let total_bytes = metadata
        .files
        .iter()
        .try_fold(0u64, |total, (name, size)| {
            fs_safety::validate_relative_path(name).ok()?;
            total.checked_add(*size)
        })?;
    (total_bytes == metadata.total_bytes).then(|| SaveSnapshotSummary {
        id: id.to_owned(),
        created_at: created_at.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        file_count: metadata.files.len(),
        total_bytes,
    })
}

fn save_snapshot_root_matches(expected: &Path, recorded: &Path) -> bool {
    if expected == recorded {
        return true;
    }
    // Match ID, snapshots, game and source exactly. Canonicalization must not
    // let two distinct qualified IDs alias each other on case-insensitive disks.
    if !expected
        .components()
        .rev()
        .take(4)
        .eq(recorded.components().rev().take(4))
    {
        return false;
    }
    // Only resolve the guarded directory obtained by listing. A relocated or
    // copied backup with a stale root is skipped, never followed or rewritten.
    let Ok(canonical) = expected.canonicalize() else {
        return false;
    };
    if canonical == recorded {
        return true;
    }
    #[cfg(windows)]
    if let Some(dos) = canonical
        .to_str()
        .and_then(|path| path.strip_prefix(r"\\?\"))
    {
        // Accept the conventional spelling of exactly this canonical DOS/UNC
        // path too, without folding case across case-sensitive directories.
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = canonical.components().next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(_) => return Path::new(dos) == recorded,
                Prefix::VerbatimUNC(..) => {
                    return dos
                        .strip_prefix(r"UNC\")
                        .is_some_and(|unc| Path::new(&format!(r"\\{unc}")) == recorded);
                }
                _ => {}
            }
        }
    }
    false
}

struct PersistenceLock {
    _file: File,
}

struct SavePath<'a> {
    declared: &'a str,
    namespace: &'static str,
    relative: String,
}

fn parse_path(path: &str) -> Result<SavePath<'_>, FusionError> {
    for (prefix, namespace) in [
        ("workspace/", "workspace"),
        ("{{APPDATA}}/", "appdata"),
        ("{{LOCALAPPDATA}}/", "localappdata"),
    ] {
        if let Some(relative) = path.strip_prefix(prefix) {
            fs_safety::validate_relative_path(relative)?;
            return Ok(SavePath {
                declared: path,
                namespace,
                relative: relative.to_owned(),
            });
        }
    }
    Err(invalid(
        "Save paths must use workspace/, {{APPDATA}}/ or {{LOCALAPPDATA}}/.",
    ))
}

fn validated_paths(spec: &PersistenceSpec) -> Result<Vec<SavePath<'_>>, FusionError> {
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for path in &spec.saves_paths {
        let parsed = parse_path(path)?;
        let identity = format!(
            "{}/{}",
            parsed.namespace,
            parsed.relative.to_ascii_lowercase()
        );
        if !seen.insert(identity.clone())
            || seen.iter().any(|other| {
                other != &identity
                    && (identity.starts_with(&format!("{other}/"))
                        || other.starts_with(&format!("{identity}/")))
            })
        {
            return Err(invalid("Save paths cannot be duplicated or nested."));
        }
        paths.push(parsed);
    }
    Ok(paths)
}

fn safe_id(value: &str) -> Result<PathBuf, FusionError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidId,
            "Invalid persistence identity.",
        ));
    }
    fs_safety::validate_relative_path(value)
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::PersistenceConflict, message)
}

fn io_error(operation: &str, path: &Path, error: std::io::Error) -> FusionError {
    FusionError::new(ErrorCode::Io, format!("Failed to {operation}."))
        .with_detail("path", path.to_string_lossy())
        .with_detail("reason", error.to_string())
}

fn new_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:032x}-{:016x}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn check_plain(path: &Path, metadata: &fs::Metadata) -> Result<(), FusionError> {
    if is_reparse(metadata) || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(invalid(
            "Save paths cannot contain symlinks, reparse points or special files.",
        )
        .with_detail("path", path.to_string_lossy()));
    }
    let _parent = fs_safety::guard(
        path.parent()
            .ok_or_else(|| invalid("Missing save parent."))?,
    )?;
    Ok(())
}

fn check_existing_ancestors(path: &Path, root: &Path) -> Result<(), FusionError> {
    let mut current = path.parent();
    while let Some(ancestor) = current {
        if ancestor.exists() {
            let _guard = fs_safety::guard(ancestor)?;
            return Ok(());
        }
        if ancestor == root {
            break;
        }
        current = ancestor.parent();
    }
    Ok(())
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    inventory: &mut BTreeMap<String, u64>,
    prefix: &str,
) -> Result<(), FusionError> {
    let _source = fs_safety::guard(source)?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err(invalid("Save copy destination already exists."));
    }
    let _destination = fs_safety::ensure_directory(destination)?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| io_error("read saves", source, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error("read save entry", source, error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("Save filenames must be valid Unicode."))?;
        fs_safety::validate_relative_path(&name)?;
        let input = entry.path();
        let output = destination.join(&name);
        let metadata = fs::symlink_metadata(&input)
            .map_err(|error| io_error("inspect save entry", &input, error))?;
        check_plain(&input, &metadata)?;
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if metadata.is_dir() {
            copy_tree(&input, &output, inventory, &relative)?;
        } else {
            fs_safety::copy_file(&input, &output)?;
            inventory.insert(relative, metadata.len());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn create_directory_link(target: &Path, link: &Path) -> Result<(), FusionError> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|error| io_error("create save directory link", link, error))
}

#[cfg(windows)]
fn create_file_link(target: &Path, link: &Path) -> Result<(), FusionError> {
    std::os::windows::fs::symlink_file(target, link).map_err(|error| {
        FusionError::new(ErrorCode::ProjectionUnavailable, "Cross-volume file saves require file-symlink permission; use a save directory when unavailable.")
            .with_detail("path", link.to_string_lossy()).with_detail("reason", error.to_string())
    })
}

#[cfg(not(windows))]
fn create_file_link(target: &Path, link: &Path) -> Result<(), FusionError> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|error| io_error("create file save link", link, error))
}

#[cfg(windows)]
fn create_directory_link(target: &Path, link: &Path) -> Result<(), FusionError> {
    use std::ffi::c_void;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(
            handle: *mut c_void,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }
    let target = fs::canonicalize(target)
        .map_err(|error| io_error("resolve save directory", target, error))?;
    let target_text = target
        .to_str()
        .ok_or_else(|| invalid("Save paths must be valid Unicode."))?;
    let printed = target_text.strip_prefix(r"\\?\").unwrap_or(target_text);
    if printed.starts_with("UNC\\") || printed.starts_with("\\\\") {
        return Err(invalid("Save junctions require a local target volume."));
    }
    let substitute: Vec<u16> = format!(r"\??\{printed}").encode_utf16().collect();
    let display: Vec<u16> = printed.encode_utf16().collect();
    let path_bytes = (substitute.len() + display.len() + 2) * 2;
    let data_length = 8 + path_bytes;
    if data_length > u16::MAX as usize || data_length + 8 > 16 * 1024 {
        return Err(invalid("Save junction target is too long."));
    }
    let mut buffer = Vec::<u8>::with_capacity(8 + data_length);
    buffer.extend_from_slice(&0xa0000003u32.to_le_bytes());
    buffer.extend_from_slice(&(data_length as u16).to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    for value in [
        0u16,
        (substitute.len() * 2) as u16,
        ((substitute.len() + 1) * 2) as u16,
        (display.len() * 2) as u16,
    ] {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    for value in substitute.into_iter().chain([0]).chain(display).chain([0]) {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    fs::create_dir(link)
        .map_err(|error| io_error("create save junction directory", link, error))?;
    let result = (|| {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(link)
            .map_err(|error| io_error("open save junction directory", link, error))?;
        let mut returned = 0;
        // SAFETY: The handle is a live directory handle; the reparse buffer has
        // the Windows mount-point layout and remains live for the synchronous call.
        let success = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                0x000900a4,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io_error(
                "set save junction target",
                link,
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir(link);
    }
    result
}

/// Unlink an explicitly owned save directory link. Never recurse into its target.
pub fn unlink_workspace_save_link(link: &Path) -> Result<(), FusionError> {
    let _parent = fs_safety::guard(
        link.parent()
            .ok_or_else(|| invalid("Missing save link parent."))?,
    )?;
    let metadata =
        fs::symlink_metadata(link).map_err(|error| io_error("inspect save link", link, error))?;
    if !is_reparse(&metadata) {
        return Err(invalid("Expected an owned save directory link."));
    }
    #[cfg(windows)]
    {
        fs::remove_dir(link).map_err(|error| io_error("unlink save junction", link, error))
    }
    #[cfg(not(windows))]
    {
        fs::remove_file(link).map_err(|error| io_error("unlink save directory link", link, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVENTORY_ROOT: &str = ".fusion-source/persistence/source/game/snapshots";

    fn inventory_game() -> QualifiedGameId {
        QualifiedGameId {
            source_id: "source".into(),
            game_id: "game".into(),
        }
    }

    fn inventory_entry(snapshots: &Path, seconds: u64) -> SaveSnapshot {
        let id = format!("{:032x}-0000000000000000", seconds * 1_000_000_000);
        SaveSnapshot {
            root: snapshots.join(&id),
            id,
            files: BTreeMap::from([("workspace/example.dat".into(), 12)]),
            total_bytes: 12,
        }
    }

    fn write_inventory_entry(path: &Path, snapshot: &SaveSnapshot) {
        fs::create_dir_all(path).unwrap();
        fs::write(
            path.join(".snapshot.json"),
            serde_json::to_vec(snapshot).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn save_inventory_missing_tree_does_not_create_it() {
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path();
        let listed = list_save_snapshots(library, &inventory_game()).unwrap();
        assert!(listed.snapshots.is_empty());
        assert_eq!(listed.skipped_count, 0);
        assert!(!listed.has_more);
        assert_eq!(fs::read_dir(library).unwrap().count(), 0);
    }

    #[test]
    fn save_inventory_caps_results_and_metadata_reads() {
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path();
        let snapshots = library.join(INVENTORY_ROOT);
        let count = MAX_LISTED_SAVE_SNAPSHOTS as u64 + 1;
        for seconds in 1..=count {
            let snapshot = inventory_entry(&snapshots, seconds);
            write_inventory_entry(&snapshot.root, &snapshot);
        }
        let listed = list_save_snapshots(library, &inventory_game()).unwrap();
        assert_eq!(listed.snapshots.len(), MAX_LISTED_SAVE_SNAPSHOTS);
        assert_eq!(
            listed.snapshots[0].id,
            inventory_entry(&snapshots, count).id
        );
        assert_eq!(
            listed.snapshots.last().unwrap().id,
            inventory_entry(&snapshots, 2).id
        );
        assert!(listed.has_more);
        assert_eq!(listed.skipped_count, 0);

        for seconds in count + 1..=count + MAX_READ_SAVE_METADATA as u64 + 1 {
            fs::create_dir(inventory_entry(&snapshots, seconds).root).unwrap();
        }
        let listed = list_save_snapshots(library, &inventory_game()).unwrap();
        assert!(listed.snapshots.is_empty());
        assert!(listed.has_more);
        assert_eq!(listed.skipped_count, MAX_READ_SAVE_METADATA);
    }

    #[test]
    fn save_inventory_isolates_bad_metadata_and_ignores_unpublished_names() {
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path();
        let snapshots = library.join(INVENTORY_ROOT);

        let good = inventory_entry(&snapshots, 1);
        let mut canonical = inventory_entry(&snapshots, 2);
        write_inventory_entry(&good.root, &good);
        fs::create_dir(&canonical.root).unwrap();
        canonical.root = canonical.root.canonicalize().unwrap();
        write_inventory_entry(&canonical.root, &canonical);

        let wrong_id = inventory_entry(&snapshots, 3);
        write_inventory_entry(&wrong_id.root, &good);
        let wrong_root = inventory_entry(&snapshots, 4);
        write_inventory_entry(
            &wrong_root.root,
            &SaveSnapshot {
                root: library.join("elsewhere").join(&wrong_root.id),
                ..wrong_root.clone()
            },
        );
        let mut wrong_total = inventory_entry(&snapshots, 5);
        wrong_total.total_bytes += 1;
        write_inventory_entry(&wrong_total.root, &wrong_total);
        let mut overflow = inventory_entry(&snapshots, 6);
        overflow
            .files
            .insert("workspace/overflow.dat".into(), u64::MAX);
        write_inventory_entry(&overflow.root, &overflow);
        let corrupt = inventory_entry(&snapshots, 7);
        fs::create_dir(&corrupt.root).unwrap();
        fs::write(corrupt.root.join(".snapshot.json"), b"not JSON").unwrap();
        let oversized = inventory_entry(&snapshots, 8);
        write_inventory_entry(&oversized.root, &oversized);
        OpenOptions::new()
            .write(true)
            .open(oversized.root.join(".snapshot.json"))
            .unwrap()
            .set_len(2 * 1024 * 1024)
            .unwrap();
        let file = inventory_entry(&snapshots, 9);
        fs::write(file.root, b"not a directory").unwrap();
        let incomplete = inventory_entry(&snapshots, 10);
        write_inventory_entry(
            &snapshots.join(format!("{}.incomplete", incomplete.id)),
            &incomplete,
        );
        write_inventory_entry(&snapshots.join("unrecognized"), &good);

        let metadata = good.root.join(".snapshot.json");
        let before = (
            fs::read(&metadata).unwrap(),
            fs::metadata(&metadata).unwrap().modified().unwrap(),
        );
        let listed = list_save_snapshots(library, &inventory_game()).unwrap();
        assert_eq!(listed.skipped_count, 7);
        assert!(!listed.has_more);
        assert_eq!(listed.snapshots.len(), 2);
        assert_eq!(listed.snapshots[0].id, canonical.id);
        assert_eq!(listed.snapshots[1].id, good.id);
        assert_eq!(listed.snapshots[1].created_at, "1970-01-01T00:00:01Z");
        assert_eq!(listed.snapshots[1].file_count, 1);
        assert_eq!(listed.snapshots[1].total_bytes, 12);
        assert_eq!(
            before,
            (
                fs::read(&metadata).unwrap(),
                fs::metadata(&metadata).unwrap().modified().unwrap()
            )
        );
        assert_eq!(
            fs::read_dir(snapshots.parent().unwrap()).unwrap().count(),
            1
        );
        // No payload fixture is needed: this only inventories published metadata.
    }

    #[test]
    fn save_inventory_rejects_reparse_entries_and_unsafe_parents() {
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path();
        let snapshots = library.join(INVENTORY_ROOT);
        let good = inventory_entry(&snapshots, 1);
        write_inventory_entry(&good.root, &good);
        let linked = inventory_entry(&snapshots, 2);
        let outside = library.join("outside");
        // Even otherwise valid metadata under the link must not be returned.
        write_inventory_entry(&outside, &linked);
        create_directory_link(&outside, &linked.root).unwrap();
        let hardlinked = inventory_entry(&snapshots, 3);
        let elsewhere = library.join("elsewhere");
        write_inventory_entry(&elsewhere, &hardlinked);
        fs::create_dir(&hardlinked.root).unwrap();
        fs::hard_link(
            elsewhere.join(".snapshot.json"),
            hardlinked.root.join(".snapshot.json"),
        )
        .unwrap();

        let listed = list_save_snapshots(library, &inventory_game()).unwrap();
        assert_eq!(listed.skipped_count, 2);
        assert_eq!(listed.snapshots.len(), 1);
        assert_eq!(listed.snapshots[0].id, good.id);
        unlink_workspace_save_link(&linked.root).unwrap();

        let source = snapshots.parent().unwrap().parent().unwrap();
        fs::rename(source, library.join("relocated-source")).unwrap();
        // The target lacks game/snapshots. Checking existence first would hide the link.
        create_directory_link(&outside, source).unwrap();
        assert!(list_save_snapshots(library, &inventory_game()).is_err());
        unlink_workspace_save_link(source).unwrap();
    }

    fn setup() -> (
        tempfile::TempDir,
        PersistenceManager,
        PersistenceRoots,
        PathBuf,
    ) {
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path().join("library");
        let workspace = library.join("version-one");
        let appdata = temporary.path().join("original-appdata");
        let local_appdata = temporary.path().join("original-localappdata");
        for path in [&workspace, &appdata, &local_appdata] {
            fs::create_dir_all(path).unwrap();
        }
        let manager = PersistenceManager::open(
            &library,
            &QualifiedGameId {
                source_id: "source".into(),
                game_id: "game".into(),
            },
        )
        .unwrap();
        (
            temporary,
            manager,
            PersistenceRoots {
                appdata,
                local_appdata,
            },
            workspace,
        )
    }

    #[test]
    fn saves_survive_workspace_changes_and_snapshot_preserves_old_bytes() {
        let (temporary, manager, roots, first) = setup();
        fs::create_dir(first.join("saves")).unwrap();
        fs::write(first.join("saves/progress.dat"), b"level one").unwrap();
        let spec = PersistenceSpec {
            saves_paths: vec!["workspace/saves".into()],
        };
        let bindings = manager.prepare(&first, &spec, &roots).unwrap();
        manager.bind_workspace(&first, &bindings).unwrap();
        fs::write(first.join("saves/progress.dat"), b"level two").unwrap();
        let snapshot = manager.snapshot_before_update().unwrap();
        assert_eq!(snapshot.total_bytes, 9);
        assert_eq!(
            fs::read(snapshot.root.join("workspace/saves/progress.dat")).unwrap(),
            b"level two"
        );
        let second = temporary.path().join("library/version-two");
        fs::create_dir(&second).unwrap();
        let next = manager.prepare(&second, &spec, &roots).unwrap();
        manager.bind_workspace(&second, &next).unwrap();
        fs::write(second.join("saves/progress.dat"), b"level three").unwrap();
        assert_eq!(
            fs::read(first.join("saves/progress.dat")).unwrap(),
            b"level three"
        );
        assert_eq!(
            fs::read(snapshot.root.join("workspace/saves/progress.dat")).unwrap(),
            b"level two"
        );
        unlink_workspace_save_link(&first.join("saves")).unwrap();
        unlink_workspace_save_link(&second.join("saves")).unwrap();
        assert_eq!(
            fs::read(bindings.bindings[0].persistent_path.join("progress.dat")).unwrap(),
            b"level three"
        );
    }

    #[test]
    fn appdata_import_preserves_originals_and_isolates_environment() {
        let (_temporary, manager, roots, workspace) = setup();
        fs::create_dir(roots.appdata.join("Game")).unwrap();
        fs::write(roots.appdata.join("Game/save.dat"), b"original").unwrap();
        let spec = PersistenceSpec {
            saves_paths: vec!["{{APPDATA}}/Game".into(), "{{LOCALAPPDATA}}/Cache".into()],
        };
        let bindings = manager.prepare(&workspace, &spec, &roots).unwrap();
        fs::write(
            bindings.bindings[0].persistent_path.join("save.dat"),
            b"private",
        )
        .unwrap();
        assert_eq!(
            fs::read(roots.appdata.join("Game/save.dat")).unwrap(),
            b"original"
        );
        assert_ne!(
            PathBuf::from(&bindings.env_overrides["APPDATA"]),
            roots.appdata
        );
        assert!(!is_reparse(&fs::symlink_metadata(&roots.appdata).unwrap()));
        let again = manager.prepare(&workspace, &spec, &roots).unwrap();
        assert_eq!(
            fs::read(again.bindings[0].persistent_path.join("save.dat")).unwrap(),
            b"private"
        );
        manager.snapshot_before_update().unwrap();
    }

    #[test]
    fn existing_file_saves_are_mutable_and_backed_up() {
        let (_temporary, manager, roots, workspace) = setup();
        fs::write(workspace.join("save.dat"), b"original").unwrap();
        let spec = PersistenceSpec {
            saves_paths: vec!["workspace/save.dat".into()],
        };
        let bindings = manager.prepare(&workspace, &spec, &roots).unwrap();
        assert_eq!(bindings.bindings[0].kind, SavePathKind::File);
        manager.bind_workspace(&workspace, &bindings).unwrap();
        fs::write(workspace.join("save.dat"), b"changed").unwrap();
        assert_eq!(
            fs::read(&bindings.bindings[0].persistent_path).unwrap(),
            b"changed"
        );
        assert_eq!(
            manager.snapshot_before_update().unwrap().files["workspace/save.dat"],
            7
        );
    }

    #[test]
    fn atomic_file_save_replacement_is_reconciled_before_backup_and_rebind() {
        let (temporary, manager, roots, workspace) = setup();
        fs::write(workspace.join("save.dat"), b"original").unwrap();
        let spec = PersistenceSpec {
            saves_paths: vec!["workspace/save.dat".into()],
        };
        let bindings = manager.prepare(&workspace, &spec, &roots).unwrap();
        manager.bind_workspace(&workspace, &bindings).unwrap();
        // Model the game's atomic replacement without writing the old inode.
        fs_safety::atomic_write(&workspace.join("save.dat"), b"latest saved progress").unwrap();
        assert_eq!(
            fs::read(&bindings.bindings[0].persistent_path).unwrap(),
            b"original"
        );
        manager.reconcile_workspace(&workspace, &bindings).unwrap();
        assert_eq!(
            fs::read(workspace.join("save.dat")).unwrap(),
            b"latest saved progress"
        );
        let snapshot = manager.snapshot_before_update().unwrap();
        assert_eq!(
            fs::read(snapshot.root.join("workspace/save.dat")).unwrap(),
            b"latest saved progress"
        );
        let next = temporary.path().join("library/version-two");
        fs::create_dir(&next).unwrap();
        let next_bindings = manager.prepare(&next, &spec, &roots).unwrap();
        manager.bind_workspace(&next, &next_bindings).unwrap();
        assert_eq!(
            fs::read(next.join("save.dat")).unwrap(),
            b"latest saved progress"
        );
        // Rebinding the old binary workspace also uses the current save slot.
        manager.bind_workspace(&workspace, &bindings).unwrap();
        assert_eq!(
            fs::read(workspace.join("save.dat")).unwrap(),
            b"latest saved progress"
        );
        manager.unbind_workspace(&workspace, &bindings).unwrap();
        manager.unbind_workspace(&next, &next_bindings).unwrap();
    }

    #[test]
    fn invalid_or_overlapping_paths_are_rejected_before_import() {
        let (_temporary, manager, roots, workspace) = setup();
        for paths in [
            vec!["workspace/../escape"],
            vec!["{{HOME}}/save"],
            vec!["workspace/saves", "workspace/saves/nested"],
            vec!["workspace/Saves", "workspace/saves"],
            vec!["workspace/CON"],
        ] {
            let spec = PersistenceSpec {
                saves_paths: paths.into_iter().map(str::to_owned).collect(),
            };
            assert!(manager.prepare(&workspace, &spec, &roots).is_err());
        }
        assert!(fs::read_dir(manager.root().join("data"))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn source_junction_cannot_be_imported_or_backed_up() {
        let (temporary, manager, roots, workspace) = setup();
        let external = temporary.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("precious"), b"keep").unwrap();
        create_directory_link(&external, &workspace.join("saves")).unwrap();
        let spec = PersistenceSpec {
            saves_paths: vec!["workspace/saves".into()],
        };
        assert!(manager.prepare(&workspace, &spec, &roots).is_err());
        unlink_workspace_save_link(&workspace.join("saves")).unwrap();
        create_directory_link(&external, &manager.root().join("data/escaped")).unwrap();
        assert!(manager.snapshot_before_update().is_err());
        assert_eq!(fs::read(external.join("precious")).unwrap(), b"keep");
        unlink_workspace_save_link(&manager.root().join("data/escaped")).unwrap();
    }

    #[test]
    fn manual_backup_accepts_canonical_link_target_and_rejects_chained_or_external_links() {
        let (temporary, manager, roots, workspace) = setup();
        let canonical_library = fs::canonicalize(temporary.path().join("library")).unwrap();
        let canonical_manager = PersistenceManager::open(
            &canonical_library,
            &QualifiedGameId {
                source_id: "source".into(),
                game_id: "game".into(),
            },
        )
        .unwrap();
        let spec = PersistenceSpec {
            saves_paths: vec!["workspace/saves".into()],
        };
        let bindings = canonical_manager
            .prepare(&workspace, &spec, &roots)
            .unwrap();
        canonical_manager
            .bind_workspace(&workspace, &bindings)
            .unwrap();
        let save_dir = workspace.join("saves");
        fs::write(save_dir.join("slot.dat"), b"canonical-alias-save").unwrap();

        canonical_manager
            .snapshot_manually(&workspace, &bindings, &Cancellation::default())
            .unwrap();

        // Reject links whose raw reparse target is itself another reparse point
        // (workspace/saves -> chain -> persistent).
        unlink_workspace_save_link(&save_dir).unwrap();
        let chain = temporary.path().join("chain");
        create_directory_link(&bindings.bindings[0].persistent_path, &chain).unwrap();
        #[cfg(windows)]
        {
            let created = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(&save_dir)
                .arg(&chain)
                .output()
                .unwrap();
            assert!(
                created.status.success(),
                "{}",
                String::from_utf8_lossy(&created.stderr)
            );
            let chained_err = canonical_manager
                .snapshot_manually(&workspace, &bindings, &Cancellation::default())
                .unwrap_err();
            assert_eq!(chained_err.code, ErrorCode::PersistenceConflict);
            unlink_workspace_save_link(&save_dir).unwrap();
        }
        unlink_workspace_save_link(&chain).unwrap();

        // Reject links pointing outside the recorded persistent save slot.
        let external = temporary.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("slot.dat"), b"outside").unwrap();
        create_directory_link(&external, &save_dir).unwrap();
        let external_err = canonical_manager
            .snapshot_manually(&workspace, &bindings, &Cancellation::default())
            .unwrap_err();
        assert_eq!(external_err.code, ErrorCode::PersistenceConflict);
        unlink_workspace_save_link(&save_dir).unwrap();

        drop(manager);
    }
}
