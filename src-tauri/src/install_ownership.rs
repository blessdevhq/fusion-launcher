//! Installation ownership comes only from the trusted application database.
//! Never deserialize this record from an archive, marker, or download directory.
//! Callers must stop all application writers before snapshotting or cleaning up.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InstallationOwnership {
    subject: String,
    attempt: String,
    root: PathBuf,
    root_identity: FileIdentity,
    files: Vec<OwnedEntry>,
    directories: Vec<OwnedEntry>,
    // Old persisted inventories must never gain new deletion authority merely
    // because they predate this field.
    #[serde(default = "sealed_inventory")]
    snapshot_taken: bool,
}

fn sealed_inventory() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OwnedEntry {
    relative: PathBuf,
    identity: FileIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileIdentity {
    device: u64,
    index: u64,
    created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanupOutcome {
    Deleted,
    Absent,
    Preserved(String),
}

#[derive(Debug, Default)]
pub(crate) struct CleanupReport {
    pub entries: Vec<(PathBuf, CleanupOutcome)>,
}

pub(crate) struct WriteProtection {
    record: InstallationOwnership,
    #[cfg(windows)]
    _guards: Vec<fs::File>,
}

pub(crate) struct DirectoryProtection {
    #[cfg(windows)]
    _guards: Vec<fs::File>,
}

/// Keeps every ancestor and the directory itself from being renamed or
/// replaced while an output is opened and written below it.
pub(crate) fn lock_plain_directory(path: &Path) -> Result<DirectoryProtection, String> {
    #[cfg(windows)]
    {
        Ok(DirectoryProtection {
            _guards: windows_identity::lock_directory(path)?,
        })
    }
    #[cfg(not(windows))]
    {
        validate_plain_path(path)?;
        if !path.is_dir() {
            return Err("Output parent is not a directory.".into());
        }
        Ok(DirectoryProtection {})
    }
}

fn create_owned_directory(path: &Path) -> std::io::Result<(FileIdentity, DirectoryProtection)> {
    #[cfg(windows)]
    {
        let file = windows_identity::create_directory(path)?;
        let identity = windows_identity::handle_identity(&file).map_err(std::io::Error::other)?;
        Ok((
            identity,
            DirectoryProtection {
                _guards: vec![file],
            },
        ))
    }
    #[cfg(not(windows))]
    {
        fs::create_dir(path)?;
        Ok((
            identity(path).map_err(std::io::Error::other)?,
            DirectoryProtection {},
        ))
    }
}

/// Opens the exact publication source and keeps it immutable until publication.
/// Callers that reopen a previously verified file must verify its bytes again
/// through this descriptor before publishing it.
pub(crate) fn open_publication_file(path: &Path) -> Result<fs::File, String> {
    validate_plain_path(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .access_mode(0x80000000 | 0x10000) // GENERIC_READ | DELETE
            .share_mode(1) // Deny writers and source replacement.
            .custom_flags(0x00200000) // OPEN_REPARSE_POINT
            .open(path)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("Handle-bound publication is only supported on Windows.".into())
    }
}

/// Moves the already verified open file without a second source-path lookup.
/// Replacement is disabled in the kernel operation, including a destination
/// created concurrently after the ancestor checks.
pub(crate) fn publish_rename_no_replace(file: &fs::File, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_identity::publish_rename_no_replace(file, destination)
    }
    #[cfg(not(windows))]
    {
        let _ = (file, destination);
        Err("Handle-bound publication is only supported on Windows.".into())
    }
}

/// Deletes the exact retained descriptor, never a second lookup of its path.
pub(crate) fn delete_open_file(file: &fs::File) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_identity::delete_open_file(file)
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        Err("Handle-bound deletion is only supported on Windows.".into())
    }
}

pub(crate) fn protect_writes(record: &InstallationOwnership) -> Result<WriteProtection, String> {
    validate_root(record)?;
    #[cfg(windows)]
    let guards = windows_identity::lock_root(record)?;
    Ok(WriteProtection {
        record: record.clone(),
        #[cfg(windows)]
        _guards: guards,
    })
}

pub(crate) fn protect_torrent_writes(
    record: &InstallationOwnership,
) -> Result<WriteProtection, String> {
    let mut protection = protect_writes(record)?;
    #[cfg(windows)]
    {
        let mut directories = record.directories.iter().collect::<Vec<_>>();
        directories.sort_by_key(|entry| entry.relative.components().count());
        for entry in directories.into_iter().chain(record.files.iter()) {
            protection
                ._guards
                .push(windows_identity::lock_entry(record, entry)?);
        }
    }
    Ok(protection)
}

impl WriteProtection {
    pub(crate) fn verify_partial(&self, path: &Path, file: &fs::File) -> Result<(), String> {
        validate_plain_path(path)?;
        let relative = path
            .strip_prefix(&self.record.root)
            .map_err(|_| "Partial outside owned installation.")?;
        let expected = self
            .record
            .files
            .iter()
            .find(|entry| entry.relative == relative)
            .ok_or("Partial has no trusted identity.")?;
        #[cfg(windows)]
        let actual = windows_identity::handle_identity(file)?;
        #[cfg(not(windows))]
        let actual = {
            let _ = file;
            identity(path)?
        };
        if actual != expected.identity {
            return Err("Partial file was replaced; refusing resume.".into());
        }
        verify_single_link(file)?;
        Ok(())
    }
}

#[cfg(test)]
impl CleanupReport {
    pub fn has_preserved_files(&self) -> bool {
        self.entries
            .iter()
            .any(|(_, outcome)| matches!(outcome, CleanupOutcome::Preserved(_)))
    }
}

impl InstallationOwnership {
    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn attempt(&self) -> &str {
        &self.attempt
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Creates a new dedicated directory. A preexisting path is never adopted,
/// regardless of its name, contents, or purported ownership marker.
pub(crate) fn allocate_directory(
    parent: &Path,
    subject: &str,
    attempt: &str,
) -> Result<InstallationOwnership, String> {
    if subject.is_empty() || attempt.is_empty() {
        return Err("Installation subject and attempt must not be empty.".into());
    }
    validate_plain_path(parent)?;
    if !parent.is_dir() {
        return Err("Installation parent must be an existing directory.".into());
    }
    let _parent_guard = lock_plain_directory(parent)?;
    let parent = parent.canonicalize().map_err(|error| error.to_string())?;
    let mut hash = DefaultHasher::new();
    (subject, attempt).hash(&mut hash);
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let (root, root_identity, _root_guard) = loop {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            "fusion-install-{:016x}-{stamp:x}-{sequence:x}",
            hash.finish()
        ));
        match create_owned_directory(&candidate) {
            Ok((identity, guard)) => break (candidate, identity, guard),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Cannot exclusively create installation {}: {error}",
                    candidate.display()
                ));
            }
        }
    };
    validate_plain_path(&root)?;
    Ok(InstallationOwnership {
        subject: subject.into(),
        attempt: attempt.into(),
        root_identity,
        root,
        files: Vec::new(),
        directories: Vec::new(),
        snapshot_taken: false,
    })
}

/// Records the exact contents of a freshly allocated directory. This is for
/// newly produced payload only, never for adopting an existing installation.
/// The first snapshot seals the inventory. Later snapshots validate existing
/// entries and can follow an exact file identity renamed within the owned root;
/// arbitrary additions remain unowned and survive cleanup.
pub(crate) fn snapshot_files(record: &mut InstallationOwnership) -> Result<(), String> {
    validate_root(record)?;
    let mut files = Vec::new();
    let mut directories = Vec::new();
    let mut pending = vec![record.root.clone()];
    while let Some(directory) = pending.pop() {
        validate_plain_path(&directory)?;
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let relative = path
                .strip_prefix(&record.root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            validate_relative(&relative)?;
            let previous = record
                .files
                .iter()
                .chain(record.directories.iter())
                .find(|old| old.relative == relative);
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if record.snapshot_taken && previous.is_none() {
                if !metadata.is_file() || validate_plain_path(&path).is_err() {
                    continue;
                }
                let actual = identity(&path)?;
                if !record.files.iter().any(|old| old.identity == actual)
                    || verify_single_link(&fs::File::open(&path).map_err(|e| e.to_string())?)
                        .is_err()
                {
                    continue;
                }
            } else {
                validate_plain_path(&path)?;
            }
            let owned = OwnedEntry {
                relative,
                identity: identity(&path)?,
            };
            if record
                .files
                .iter()
                .chain(record.directories.iter())
                .any(|old| old.relative == owned.relative && old.identity != owned.identity)
            {
                return Err("Installation entry was replaced; refusing to adopt it.".into());
            }
            if path.extension().is_some_and(|ext| ext == "part") {
                verify_single_link(&fs::File::open(&path).map_err(|e| e.to_string())?)?;
                if record
                    .files
                    .iter()
                    .any(|old| old.relative == owned.relative && old.identity != owned.identity)
                {
                    return Err("Partial file was replaced; refusing to adopt it.".into());
                }
            }
            if metadata.is_dir() {
                pending.push(path);
                directories.push(owned);
            } else if metadata.is_file() {
                files.push(owned);
            } else {
                return Err(format!(
                    "Unsupported installation entry: {}",
                    path.display()
                ));
            }
        }
    }
    // Recheck after enumeration; never publish a manifest for a replaced root.
    validate_root(record)?;
    record.files = files;
    record.directories = directories;
    record.snapshot_taken = true;
    Ok(())
}

fn verify_single_link(file: &fs::File) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_identity::verify_single_link(file)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(|e| e.to_string())?.nlink() != 1 {
            Err("Partial has multiple hard links.".into())
        } else {
            Ok(())
        }
    }
}

/// Reserves producer-declared paths before allowing the producer to write.
/// Existing unknown entries are never adopted. Partial progress stays in the
/// manifest so the caller can persist cleanup authority even on an error.
pub(crate) fn reserve_producer_files(
    record: &mut InstallationOwnership,
    paths: &[String],
) -> Result<(), String> {
    let _root_guard = protect_writes(record)?;
    record.snapshot_taken = true;
    if paths.len() > 100_000 {
        return Err("Torrent inventory exceeds 100000 files.".into());
    }
    for raw in paths {
        let relative = PathBuf::from(raw);
        validate_relative(&relative)?;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err("Unsafe torrent path.".into());
            };
            let name = name.to_string_lossy();
            let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
            if name.contains(':')
                || name.ends_with(['.', ' '])
                || name.chars().any(|c| c.is_control())
                || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && base.as_bytes()[3].is_ascii_digit())
            {
                return Err("Torrent inventory contains an unsafe Windows filename.".into());
            }
        }
        let mut parent = record.root.clone();
        let mut guards = Vec::new();
        if let Some(relative_parent) = relative.parent() {
            for component in relative_parent.components() {
                parent.push(component);
                let dir_relative = parent
                    .strip_prefix(&record.root)
                    .map_err(|e| e.to_string())?
                    .to_path_buf();
                match create_owned_directory(&parent) {
                    Ok((identity, guard)) => {
                        let entry = OwnedEntry {
                            relative: dir_relative.clone(),
                            identity,
                        };
                        record
                            .directories
                            .retain(|old| old.relative != dir_relative);
                        record.directories.push(entry);
                        guards.push(guard);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let expected=record.directories.iter().find(|old|old.relative==dir_relative).ok_or("Torrent output directory already exists without ownership; files preserved.")?;
                        if identity(&parent)? != expected.identity {
                            return Err(
                                "Torrent output directory was replaced; files preserved.".into()
                            );
                        }
                    }
                    Err(error) => return Err(error.to_string()),
                }
                guards.push(lock_plain_directory(&parent)?);
            }
        }
        let path = record.root.join(&relative);
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        match options.open(&path) {
            Ok(file) => {
                #[cfg(windows)]
                let actual = windows_identity::handle_identity(&file)?;
                #[cfg(not(windows))]
                let actual = identity(&path)?;
                record.files.retain(|old| old.relative != relative);
                record.files.push(OwnedEntry {
                    relative,
                    identity: actual,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                validate_plain_path(&path)?;
                let expected=record.files.iter().find(|old|old.relative==relative).ok_or("Torrent output file already exists without ownership; existing file preserved.")?;
                let file = fs::File::open(&path).map_err(|e| e.to_string())?;
                #[cfg(windows)]
                let actual = windows_identity::handle_identity(&file)?;
                #[cfg(not(windows))]
                let actual = identity(&path)?;
                if actual != expected.identity {
                    return Err("Torrent output file was replaced; existing file preserved.".into());
                }
                verify_single_link(&file)?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// Transfers a trusted installation record to a retry only after the caller has
/// confirmed the previous worker has stopped. Filesystem identity must still
/// match; neither the source payload nor a marker can authorize this transfer.
pub(crate) fn renew_attempt(
    record: &mut InstallationOwnership,
    subject: &str,
    new_attempt: &str,
) -> Result<(), String> {
    if record.subject != subject || new_attempt.is_empty() {
        return Err("Installation retry subject or attempt is invalid.".into());
    }
    validate_root(record)?;
    record.attempt = new_attempt.into();
    Ok(())
}

/// Removes exact manifest files, then empty manifest directories. Unknown files
/// (including marker files) are preserved, and no recursive deletion is used.
pub(crate) fn cleanup_files(record: &InstallationOwnership) -> CleanupReport {
    let mut report = CleanupReport::default();
    match validate_root(record) {
        Ok(()) => {}
        Err(reason) => {
            let outcome = match fs::symlink_metadata(&record.root) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    CleanupOutcome::Absent
                }
                _ => CleanupOutcome::Preserved(reason),
            };
            report.entries.push((record.root.clone(), outcome));
            return report;
        }
    }
    for entry in &record.files {
        report.entries.push((
            record.root.join(&entry.relative),
            remove_owned_entry(record, entry, false),
        ));
    }
    let mut directories = record.directories.iter().collect::<Vec<_>>();
    directories.sort_by_key(|entry| std::cmp::Reverse(entry.relative.components().count()));
    for entry in directories {
        report.entries.push((
            record.root.join(&entry.relative),
            remove_owned_entry(record, entry, true),
        ));
    }
    let root_entry = OwnedEntry {
        relative: PathBuf::new(),
        identity: record.root_identity.clone(),
    };
    report.entries.push((
        record.root.clone(),
        remove_owned_entry(record, &root_entry, true),
    ));
    report
}

fn remove_owned_entry(
    record: &InstallationOwnership,
    entry: &OwnedEntry,
    directory: bool,
) -> CleanupOutcome {
    let path = record.root.join(&entry.relative);
    let remove = || -> Result<(), String> {
        if !entry.relative.as_os_str().is_empty() {
            validate_relative(&entry.relative)?;
        }
        validate_root(record)?;
        validate_plain_path(&path)?;
        if identity(&path)? != entry.identity {
            return Err("Filesystem entry was replaced; ownership is no longer proven.".into());
        }
        remove_exact(record, entry, directory)
    };
    match remove() {
        Ok(()) => CleanupOutcome::Deleted,
        Err(reason) => match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CleanupOutcome::Absent,
            _ => CleanupOutcome::Preserved(reason),
        },
    }
}

fn validate_root(record: &InstallationOwnership) -> Result<(), String> {
    validate_plain_path(&record.root)?;
    if !record.root.is_dir() || identity(&record.root)? != record.root_identity {
        return Err("Installation directory was replaced; ownership is no longer proven.".into());
    }
    Ok(())
}

fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Owned entry must be a nonempty relative path without traversal.".into());
    }
    Ok(())
}

fn validate_plain_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Installation path must be absolute and have no traversal.".into());
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(format!(
                "Installation path contains a link: {}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn identity(path: &Path) -> Result<FileIdentity, String> {
    windows_identity::identity(path)
}

#[cfg(windows)]
fn remove_exact(
    record: &InstallationOwnership,
    entry: &OwnedEntry,
    directory: bool,
) -> Result<(), String> {
    windows_identity::remove_exact(record, entry, directory)
}

#[cfg(not(windows))]
fn remove_exact(
    record: &InstallationOwnership,
    entry: &OwnedEntry,
    directory: bool,
) -> Result<(), String> {
    let path = record.root.join(&entry.relative);
    if directory {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|error| error.to_string())
}

#[cfg(windows)]
mod windows_identity {
    use super::*;

    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    const BACKUP_SEMANTICS: u32 = 0x02000000;
    const OPEN_REPARSE_POINT: u32 = 0x00200000;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_READ_DATA_OR_LIST_DIRECTORY: u32 = 0x1;
    const DELETE: u32 = 0x10000;
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root_directory: *mut std::ffi::c_void,
        object_name: *mut UnicodeString,
        attributes: u32,
        security_descriptor: *mut std::ffi::c_void,
        security_quality_of_service: *mut std::ffi::c_void,
    }
    #[repr(C)]
    #[derive(Default)]
    struct IoStatusBlock {
        status_or_pointer: usize,
        information: usize,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtCreateFile(
            handle: *mut *mut std::ffi::c_void,
            desired_access: u32,
            attributes: *mut ObjectAttributes,
            io_status: *mut IoStatusBlock,
            allocation_size: *const i64,
            file_attributes: u32,
            share_access: u32,
            create_disposition: u32,
            create_options: u32,
            ea_buffer: *const std::ffi::c_void,
            ea_length: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }

    /// FILE_CREATE returns the newly created directory's handle atomically.
    /// Denying DELETE sharing closes the create-directory / reopen identity race.
    pub(super) fn create_directory(path: &Path) -> std::io::Result<fs::File> {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("Missing parent"))?;
        let parent = open(parent, false).map_err(std::io::Error::other)?;
        let mut name = path
            .file_name()
            .ok_or_else(|| std::io::Error::other("Missing name"))?
            .encode_wide()
            .collect::<Vec<_>>();
        let length = u16::try_from(name.len() * 2).map_err(std::io::Error::other)?;
        let mut unicode = UnicodeString {
            length,
            maximum_length: length,
            buffer: name.as_mut_ptr(),
        };
        let mut attributes = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root_directory: parent.as_raw_handle(),
            object_name: &mut unicode,
            attributes: 0x40, // OBJ_CASE_INSENSITIVE; no handle inheritance.
            security_descriptor: std::ptr::null_mut(),
            security_quality_of_service: std::ptr::null_mut(),
        };
        let mut handle = std::ptr::null_mut();
        let mut io_status = IoStatusBlock::default();
        // SAFETY: all structures and UTF-16 name remain live for this synchronous
        // call; RootDirectory is a validated directory with denied DELETE share.
        let status = unsafe {
            NtCreateFile(
                &mut handle,
                FILE_READ_ATTRIBUTES | FILE_READ_DATA_OR_LIST_DIRECTORY | 0x100000,
                &mut attributes,
                &mut io_status,
                std::ptr::null(),
                0x10,
                SHARE_READ_WRITE,
                2,                               // FILE_CREATE, never open/adopt an existing entry.
                0x1 | 0x20 | OPEN_REPARSE_POINT, // DIRECTORY | SYNCHRONOUS_IO_NONALERT
                std::ptr::null(),
                0,
            )
        };
        if status < 0 {
            return Err(std::io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(status) } as i32,
            ));
        }
        // SAFETY: successful NtCreateFile transferred a unique owned handle.
        Ok(unsafe { fs::File::from_raw_handle(handle) })
    }

    #[repr(C)]
    #[derive(Default)]
    struct FileInformation {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            information: *mut FileInformation,
        ) -> i32;
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            information: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    fn open(path: &Path, delete_access: bool) -> Result<fs::File, String> {
        fs::OpenOptions::new()
            .read(true)
            // Attribute-only handles do not participate in Windows sharing
            // checks. Actual read/list access makes denied DELETE sharing
            // prevent an ancestor from being renamed or replaced.
            .access_mode(
                FILE_READ_ATTRIBUTES
                    | FILE_READ_DATA_OR_LIST_DIRECTORY
                    | if delete_access { DELETE } else { 0 },
            )
            .share_mode(SHARE_READ_WRITE) // Deny renaming/replacing the opened entry.
            .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|error| format!("Cannot lock {}: {error}", path.display()))
    }

    pub(super) fn handle_identity(file: &fs::File) -> Result<FileIdentity, String> {
        let mut information = FileInformation::default();
        // SAFETY: valid live file handle and correctly laid out writable structure.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if information.attributes & 0x400 != 0 {
            return Err("Filesystem entry is a reparse point.".into());
        }
        Ok(FileIdentity {
            device: u64::from(information.volume),
            index: (u64::from(information.index_high) << 32) | u64::from(information.index_low),
            created: (u64::from(information.creation[1]) << 32)
                | u64::from(information.creation[0]),
        })
    }

    pub(super) fn identity(path: &Path) -> Result<FileIdentity, String> {
        handle_identity(&open(path, false)?)
    }

    pub(super) fn verify_single_link(file: &fs::File) -> Result<(), String> {
        let mut info = FileInformation::default();
        // SAFETY: a live file handle and a correctly laid out writable structure.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if info.links != 1 {
            return Err("Partial has multiple hard links.".into());
        }
        Ok(())
    }

    pub(super) fn lock_root(record: &InstallationOwnership) -> Result<Vec<fs::File>, String> {
        let mut paths = record.root.ancestors().collect::<Vec<_>>();
        paths.reverse();
        let mut guards = Vec::new();
        for path in paths {
            let handle = open(path, false)?;
            let actual = handle_identity(&handle)?;
            if !handle.metadata().map_err(|e| e.to_string())?.is_dir() {
                return Err("Installation ancestor is not a directory.".into());
            }
            if path == record.root && actual != record.root_identity {
                return Err("Installation root was replaced.".into());
            }
            guards.push(handle);
        }
        Ok(guards)
    }

    pub(super) fn lock_entry(
        record: &InstallationOwnership,
        entry: &OwnedEntry,
    ) -> Result<fs::File, String> {
        validate_relative(&entry.relative)?;
        let path = record.root.join(&entry.relative);
        let file = open(&path, false)?;
        if handle_identity(&file)? != entry.identity {
            return Err("Torrent inventory entry was replaced; files preserved.".into());
        }
        if file.metadata().map_err(|e| e.to_string())?.is_file() {
            verify_single_link(&file)?;
        }
        Ok(file)
    }

    pub(super) fn publish_rename_no_replace(
        file: &fs::File,
        destination: &Path,
    ) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Publication source is not a regular file.".into());
        }
        handle_identity(file)?; // Reject an opened reparse point.
        let parent = destination
            .parent()
            .ok_or("Publication destination has no parent.")?;
        let name = destination
            .file_name()
            .ok_or("Publication destination has no filename.")?;
        let _guards = lock_directory(parent)?;
        let destination = parent.canonicalize().map_err(|e| e.to_string())?.join(name);
        let name = destination.as_os_str().encode_wide().collect::<Vec<_>>();
        #[repr(C)]
        struct RenameInformation {
            replace_if_exists: u32,
            root_directory: *mut std::ffi::c_void,
            file_name_length: u32,
            file_name: [u16; 0],
        }
        let offset = std::mem::offset_of!(RenameInformation, file_name);
        let bytes = name
            .len()
            .checked_mul(2)
            .ok_or("Publication path is too long.")?;
        let length = offset
            .checked_add(bytes)
            .ok_or("Publication path is too long.")?;
        let size = u32::try_from(length).map_err(|_| "Publication path is too long.")?;
        // usize backing gives the structure its required HANDLE alignment.
        let mut buffer = vec![0usize; length.div_ceil(std::mem::size_of::<usize>())];
        let information = buffer.as_mut_ptr().cast::<RenameInformation>();
        // SAFETY: aligned allocation covers the header and the complete UTF-16
        // filename, and remains live through the synchronous kernel call.
        let success = unsafe {
            (*information).replace_if_exists = 0;
            (*information).root_directory = std::ptr::null_mut();
            (*information).file_name_length = bytes as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                buffer.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
                name.len(),
            );
            SetFileInformationByHandle(file.as_raw_handle(), 3, information.cast(), size)
        };
        if success == 0 {
            return Err(format!(
                "Failed to publish without overwriting files: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub(super) fn delete_open_file(file: &fs::File) -> Result<(), String> {
        handle_identity(file)?;
        let delete_file: i32 = 1;
        // SAFETY: the retained descriptor has DELETE access, and class 4 takes
        // FILE_DISPOSITION_INFO containing exactly one Windows BOOL.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                4,
                std::ptr::from_ref(&delete_file).cast(),
                std::mem::size_of_val(&delete_file) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    pub(super) fn lock_directory(path: &Path) -> Result<Vec<fs::File>, String> {
        let mut parents = path.ancestors().collect::<Vec<_>>();
        parents.reverse();
        let mut guards = Vec::new();
        for ancestor in parents {
            let guard = open(ancestor, false)?;
            handle_identity(&guard)?;
            if !guard.metadata().map_err(|e| e.to_string())?.is_dir() {
                return Err("Output ancestor is not a directory.".into());
            }
            guards.push(guard);
        }
        Ok(guards)
    }

    pub(super) fn remove_exact(
        record: &InstallationOwnership,
        entry: &OwnedEntry,
        directory: bool,
    ) -> Result<(), String> {
        let path = record.root.join(&entry.relative);
        let mut parents = path
            .parent()
            .ok_or("Installation entry has no parent.")?
            .ancestors()
            .collect::<Vec<_>>();
        parents.reverse();
        // Lock ancestors from the filesystem root down before opening children.
        // This closes check/remove junction and rename races on Windows.
        let mut guards = Vec::new();
        for parent in parents {
            let handle = open(parent, false)?;
            let parent_identity = handle_identity(&handle)?;
            if parent == record.root && parent_identity != record.root_identity {
                return Err("Installation root was replaced.".into());
            }
            if !handle
                .metadata()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                return Err("Installation ancestor is not a directory.".into());
            }
            guards.push(handle);
        }
        let handle = open(&path, true)?;
        if handle_identity(&handle)? != entry.identity {
            return Err("Filesystem entry was replaced.".into());
        }
        if handle
            .metadata()
            .map_err(|error| error.to_string())?
            .is_dir()
            != directory
        {
            return Err("Filesystem entry type changed.".into());
        }
        let delete_file: i32 = 1; // FILE_DISPOSITION_INFO.DeleteFile (BOOL).
                                  // SAFETY: the validated live handle has DELETE access; class 4 expects
                                  // FILE_DISPOSITION_INFO containing one BOOL. Deletion targets this
                                  // exact handle, never a second path lookup that could be substituted.
        let success = unsafe {
            SetFileInformationByHandle(
                handle.as_raw_handle(),
                4,
                std::ptr::from_ref(&delete_file).cast(),
                std::mem::size_of_val(&delete_file) as u32,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        drop(handle);
        drop(guards);
        Ok(())
    }
}

#[cfg(unix)]
fn identity(path: &Path) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    Ok(FileIdentity {
        device: metadata.dev(),
        index: metadata.ino(),
        created: metadata
            .created()
            .ok()
            .and_then(|created| created.duration_since(UNIX_EPOCH).ok())
            .map(|created| created.as_nanos() as u64)
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn exclusive_directory_creation_returns_a_locked_identity_without_reopen() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = parent.path().join("exclusive");
        let (created, guard) = create_owned_directory(&path).unwrap();
        assert_eq!(created, identity(&path).unwrap());
        assert!(fs::rename(&path, parent.path().join("substitute")).is_err());
        assert_eq!(
            create_owned_directory(&path).err().unwrap().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        drop(guard);
        fs::rename(&path, parent.path().join("substitute")).unwrap();
    }

    #[test]
    fn producer_inventory_can_extend_a_sealed_manifest_without_adopting_collisions() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "torrent", "attempt").unwrap();
        snapshot_files(&mut record).unwrap();
        reserve_producer_files(&mut record, &["payload/first.bin".into()]).unwrap();
        fs::write(record.root().join("payload/first.bin"), b"partial").unwrap();
        reserve_producer_files(&mut record, &["payload/second.bin".into()]).unwrap();
        fs::write(record.root().join("sentinel"), b"foreign").unwrap();
        assert!(reserve_producer_files(&mut record, &["sentinel".into()]).is_err());
        assert!(reserve_producer_files(&mut record, &["../escape".into()]).is_err());
        assert!(reserve_producer_files(&mut record, &["payload/NUL.bin".into()]).is_err());
        assert!(cleanup_files(&record).has_preserved_files());
        assert_eq!(
            fs::read(record.root().join("sentinel")).unwrap(),
            b"foreign"
        );
        assert!(!record.root().join("payload").exists());
    }

    #[cfg(windows)]
    #[test]
    fn torrent_guards_lock_nested_names_allow_writes_and_release_all_ancestors() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let library = parent.path().join("library");
        fs::create_dir(&library).unwrap();
        let mut record = allocate_directory(&library, "torrent", "attempt").unwrap();
        reserve_producer_files(&mut record, &["nested/game.bin".into()]).unwrap();
        let guard = protect_torrent_writes(&record).unwrap();
        let payload = record.root().join("nested/game.bin");
        fs::write(&payload, b"downloaded").unwrap();
        assert!(fs::rename(&payload, record.root().join("replacement.bin")).is_err());
        assert!(fs::remove_file(&payload).is_err());
        assert!(fs::rename(record.root().join("nested"), record.root().join("moved")).is_err());
        assert!(fs::rename(&library, parent.path().join("renamed")).is_err());
        drop(guard);
        assert!(!cleanup_files(&record).has_preserved_files());
        fs::rename(&library, parent.path().join("renamed")).unwrap();
        fs::remove_dir(parent.path().join("renamed")).unwrap();
    }

    #[test]
    fn sealed_snapshot_never_adopts_later_unknown_additions() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        fs::write(record.root().join("game.rom"), "owned").unwrap();
        snapshot_files(&mut record).unwrap();
        fs::write(record.root().join("sentinel"), "foreign").unwrap();
        fs::create_dir(record.root().join("foreign-directory")).unwrap();
        fs::write(record.root().join("foreign-directory/file"), "foreign").unwrap();
        snapshot_files(&mut record).unwrap();
        assert!(cleanup_files(&record).has_preserved_files());
        assert_eq!(
            fs::read_to_string(record.root().join("sentinel")).unwrap(),
            "foreign"
        );
        assert!(record.root().join("foreign-directory/file").is_file());
        assert!(!record.root().join("game.rom").exists());
    }

    #[test]
    fn sealed_snapshot_tracks_a_verified_partial_identity_after_rename() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        fs::write(record.root().join("game.part"), "owned").unwrap();
        snapshot_files(&mut record).unwrap();
        fs::rename(
            record.root().join("game.part"),
            record.root().join("game.rom"),
        )
        .unwrap();
        snapshot_files(&mut record).unwrap();
        assert!(!cleanup_files(&record).has_preserved_files());
        assert!(!record.root().exists());
    }

    #[test]
    fn old_persisted_inventory_defaults_to_sealed() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let record = allocate_directory(parent.path(), "a", "1").unwrap();
        let mut value = serde_json::to_value(record).unwrap();
        value.as_object_mut().unwrap().remove("snapshot_taken");
        let mut restored: InstallationOwnership = serde_json::from_value(value).unwrap();
        fs::write(restored.root().join("sentinel"), "foreign").unwrap();
        snapshot_files(&mut restored).unwrap();
        assert!(cleanup_files(&restored).has_preserved_files());
        assert!(restored.root().join("sentinel").exists());
    }

    #[cfg(windows)]
    #[test]
    fn descriptor_deletion_cannot_follow_a_source_substitution() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = parent.path().join("archive.zip");
        fs::write(&path, b"verified").unwrap();
        let file = open_publication_file(&path).unwrap();
        assert!(fs::rename(&path, parent.path().join("original")).is_err());
        assert!(fs::write(&path, b"foreign").is_err());
        delete_open_file(&file).unwrap();
        drop(file);
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn directory_guard_blocks_parent_substitution_until_output_is_closed() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let root = parent.path().join("output");
        fs::create_dir(&root).unwrap();
        let guard = lock_plain_directory(&root).unwrap();
        assert!(fs::rename(&root, parent.path().join("moved")).is_err());
        fs::write(root.join("payload"), b"owned").unwrap();
        drop(guard);
        fs::rename(&root, parent.path().join("moved")).unwrap();
    }

    #[test]
    fn repeated_snapshot_rejects_replaced_regular_payload() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        let payload = record.root().join("game.rom");
        fs::write(&payload, "owned").unwrap();
        snapshot_files(&mut record).unwrap();
        fs::rename(&payload, parent.path().join("original")).unwrap();
        fs::write(&payload, "foreign").unwrap();
        assert!(snapshot_files(&mut record).is_err());
        assert!(cleanup_files(&record).has_preserved_files());
        assert_eq!(fs::read_to_string(payload).unwrap(), "foreign");
    }

    #[test]
    fn retry_requires_same_subject_and_original_root_identity() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        fs::write(record.root().join("download.part"), "partial").unwrap();
        renew_attempt(&mut record, "a", "2").unwrap();
        assert_eq!(record.subject(), "a");
        assert_eq!(record.attempt(), "2");
        assert!(record.root().join("download.part").is_file());
        assert!(renew_attempt(&mut record, "other", "3").is_err());
        assert!(renew_attempt(&mut record, "a", "").is_err());
        fs::rename(record.root(), parent.path().join("original")).unwrap();
        fs::create_dir(record.root()).unwrap();
        assert!(renew_attempt(&mut record, "a", "3").is_err());
        assert_eq!(record.attempt(), "2");
    }

    #[test]
    fn dedicated_directory_preserves_custom_target_and_forged_marker() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        fs::write(parent.path().join("sentinel"), "foreign").unwrap();
        fs::write(parent.path().join(".fusion-owned"), "forged").unwrap();
        let mut record = allocate_directory(parent.path(), "game-a", "attempt-a").unwrap();
        fs::create_dir(record.root().join("payload")).unwrap();
        fs::write(record.root().join("payload/game.rom"), "game").unwrap();
        snapshot_files(&mut record).unwrap();
        let cleanup = cleanup_files(&record);
        assert!(!cleanup.has_preserved_files(), "{cleanup:?}");
        assert_eq!(
            fs::read_to_string(parent.path().join("sentinel")).unwrap(),
            "foreign"
        );
        assert!(parent.path().join(".fusion-owned").is_file());
        assert!(!record.root().exists());
        assert!(cleanup_files(&record)
            .entries
            .iter()
            .all(|(_, outcome)| *outcome == CleanupOutcome::Absent));
    }

    #[test]
    fn collision_is_never_adopted_and_same_filename_is_isolated() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut first = allocate_directory(parent.path(), "a", "1").unwrap();
        fs::write(first.root().join("same.rom"), "a").unwrap();
        fs::write(first.root().join(".fusion-owned"), "forged").unwrap();
        let retry = allocate_directory(parent.path(), "a", "1").unwrap();
        assert_ne!(retry.root(), first.root());
        let second = allocate_directory(parent.path(), "b", "1").unwrap();
        fs::write(second.root().join("same.rom"), "b").unwrap();
        snapshot_files(&mut first).unwrap();
        cleanup_files(&first);
        assert_eq!(
            fs::read_to_string(second.root().join("same.rom")).unwrap(),
            "b"
        );
    }

    #[test]
    fn unlisted_file_survives_and_cleanup_is_repeatable() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        fs::write(record.root().join("game.rom"), "game").unwrap();
        snapshot_files(&mut record).unwrap();
        fs::write(record.root().join("sentinel"), "foreign").unwrap();
        assert!(cleanup_files(&record).has_preserved_files());
        assert!(record.root().join("sentinel").exists());
        assert!(!record.root().join("game.rom").exists());
        assert!(cleanup_files(&record).has_preserved_files());
    }

    #[test]
    fn replaced_root_and_payload_are_preserved() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        fs::write(record.root().join("game.rom"), "game").unwrap();
        snapshot_files(&mut record).unwrap();
        // Keep originals alive to prevent filesystem inode reuse in the test.
        fs::rename(
            record.root().join("game.rom"),
            parent.path().join("original"),
        )
        .unwrap();
        fs::write(record.root().join("game.rom"), "foreign").unwrap();
        assert!(cleanup_files(&record).has_preserved_files());
        assert_eq!(
            fs::read_to_string(record.root().join("game.rom")).unwrap(),
            "foreign"
        );
        fs::rename(record.root(), parent.path().join("old-root")).unwrap();
        fs::create_dir(record.root()).unwrap();
        fs::write(record.root().join("game.rom"), "another foreign").unwrap();
        assert!(cleanup_files(&record).has_preserved_files());
        assert!(record.root().join("game.rom").exists());
    }

    #[test]
    fn invalid_manifest_paths_are_preserved() {
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        let sentinel = parent.path().join("sentinel");
        fs::write(&sentinel, "foreign").unwrap();
        record.files.push(OwnedEntry {
            relative: PathBuf::from("../sentinel"),
            identity: identity(&sentinel).unwrap(),
        });
        assert!(cleanup_files(&record).has_preserved_files());
        assert!(sentinel.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_parent_and_payload_are_rejected() {
        use std::os::unix::fs::symlink;
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let foreign = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        symlink(foreign.path(), parent.path().join("link")).unwrap();
        assert!(allocate_directory(&parent.path().join("link"), "a", "1").is_err());
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        symlink(foreign.path(), record.root().join("payload")).unwrap();
        assert!(snapshot_files(&mut record).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn junction_parent_and_payload_are_rejected() {
        use std::os::windows::process::CommandExt;
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let foreign = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let junction = parent.path().join("junction");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(foreign.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(allocate_directory(&junction, "a", "1").is_err());
        let mut record = allocate_directory(parent.path(), "a", "1").unwrap();
        let payload_junction = record.root().join("payload");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&payload_junction)
            .arg(foreign.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(snapshot_files(&mut record).is_err());
        fs::remove_dir(payload_junction).unwrap();
        fs::remove_dir(junction).unwrap();
    }
}
