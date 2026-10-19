//! Filesystem primitives for explicitly owned Fusion Source data.
//!
//! These synchronous helpers belong on a blocking thread in a future IPC adapter.
//! Directory guards retain the launcher's existing no-reparse protections. Never
//! clear a read-only attribute while removing a linked file: it belongs to every
//! hardlink, including the immutable CAS object.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::errors::{ErrorCode, FusionError};

pub struct DirectoryGuard {
    _guards: Vec<crate::install_ownership::DirectoryProtection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume: u64,
    pub index: u64,
}

pub(crate) fn io_error(operation: &str, path: &Path, error: impl std::fmt::Display) -> FusionError {
    FusionError::new(ErrorCode::Io, format!("{operation}: {error}"))
        .with_detail("path", path.to_string_lossy())
}

pub fn validate_relative_path(value: &str) -> Result<PathBuf, FusionError> {
    super::evolution::validate_relative_path(value, false, false, "filesystem.path")?;
    Ok(PathBuf::from(value))
}

fn validate_absolute(path: &Path) -> Result<(), FusionError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidPath,
            "An absolute filesystem path without traversal is required.",
        ));
    }
    Ok(())
}

pub(crate) fn check_metadata(metadata: &fs::Metadata, path: &Path) -> Result<(), FusionError> {
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink() || reparse {
        return Err(FusionError::new(
            ErrorCode::InvalidPath,
            "Filesystem links and reparse points are not permitted here.",
        )
        .with_detail("path", path.to_string_lossy()));
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(FusionError::new(
            ErrorCode::InvalidPath,
            "Only plain files and directories are supported.",
        ));
    }
    Ok(())
}

pub fn guard(path: &Path) -> Result<DirectoryGuard, FusionError> {
    validate_absolute(path)?;
    let protection = crate::install_ownership::lock_plain_directory(path)
        .map_err(|error| io_error("Lock directory", path, error))?;
    Ok(DirectoryGuard {
        _guards: vec![protection],
    })
}

/// Creates components one at a time, keeping each existing ancestor locked.
pub fn ensure_directory(path: &Path) -> Result<DirectoryGuard, FusionError> {
    validate_absolute(path)?;
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    let mut protections = Vec::new();
    for ancestor in ancestors {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                check_metadata(&metadata, ancestor)?;
                if !metadata.is_dir() {
                    return Err(FusionError::new(
                        ErrorCode::InvalidPath,
                        "A filesystem ancestor is not a directory.",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(ancestor) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(io_error("Create directory", ancestor, error)),
                }
            }
            Err(error) => return Err(io_error("Inspect directory", ancestor, error)),
        }
        protections.push(
            crate::install_ownership::lock_plain_directory(ancestor)
                .map_err(|error| io_error("Lock directory", ancestor, error))?,
        );
    }
    Ok(DirectoryGuard {
        _guards: protections,
    })
}

/// Opens the leaf without following reparse points. On Windows, actual read
/// access and FILE_SHARE_READ exclude concurrent writers and replacements.
pub fn open_read(path: &Path) -> Result<File, FusionError> {
    let _parent = guard(
        path.parent()
            .ok_or_else(|| FusionError::new(ErrorCode::InvalidPath, "File has no parent."))?,
    )?;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("Inspect file", path, error))?;
    check_metadata(&metadata, path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NOFOLLOW; ancestors have already been checked by the directory guard.
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20000);
    }
    let file = options
        .open(path)
        .map_err(|error| io_error("Open file", path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| io_error("Inspect open file", path, error))?;
    check_metadata(&metadata, path)?;
    if !metadata.is_file() {
        return Err(FusionError::new(
            ErrorCode::InvalidPath,
            "Expected a regular file.",
        ));
    }
    Ok(file)
}

fn create_new(path: &Path) -> Result<File, FusionError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x80000000 | 0x40000000 | 0x10000)
            .share_mode(1)
            .custom_flags(0x00200000);
    }
    options
        .open(path)
        .map_err(|error| io_error("Create owned file", path, error))
}

pub fn copy_file(source: &Path, destination: &Path) -> Result<(), FusionError> {
    let _source_parent = guard(
        source
            .parent()
            .ok_or_else(|| FusionError::new(ErrorCode::InvalidPath, "Source has no parent."))?,
    )?;
    let mut source_file = open_read(source)?;
    let _parent =
        ensure_directory(destination.parent().ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidPath, "Destination has no parent.")
        })?)?;
    let mut destination_file = create_new(destination)?;
    let result = std::io::copy(&mut source_file, &mut destination_file)
        .and_then(|_| destination_file.sync_all())
        .map_err(|error| io_error("Copy file", destination, error));
    if result.is_err() {
        #[cfg(windows)]
        let _ = windows::delete_handle(&destination_file);
        #[cfg(not(windows))]
        let _ = fs::remove_file(destination);
    }
    result
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn unique_name(prefix: &str) -> String {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}-{}-{time}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FusionError> {
    let parent = path
        .parent()
        .ok_or_else(|| FusionError::new(ErrorCode::InvalidPath, "Output has no parent."))?;
    let _guard = ensure_directory(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        check_metadata(&metadata, path)?;
        if !metadata.is_file() {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Atomic output is not a regular file.",
            ));
        }
    }
    let temporary = parent.join(unique_name(".write"));
    let mut file = create_new(&temporary)?;
    let result = file
        .write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| io_error("Write atomic file", path, error))
        .and_then(|_| {
            #[cfg(windows)]
            {
                windows::rename_handle(&file, path, true)
                    .map_err(|error| io_error("Replace atomic file", path, error))
            }
            #[cfg(not(windows))]
            {
                fs::rename(&temporary, path)
                    .map_err(|error| io_error("Replace atomic file", path, error))
            }
        });
    if result.is_err() {
        #[cfg(windows)]
        let _ = windows::delete_handle(&file);
        #[cfg(not(windows))]
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), FusionError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| io_error("Serialize metadata", path, error))?;
    atomic_write(path, &bytes)
}

/// Stream into a private staging file, then replace the destination atomically.
/// Used for mutable save reconciliation; neither source nor old target is chmod'd.
pub fn atomic_copy_file(source: &Path, destination: &Path) -> Result<(), FusionError> {
    let _source_parent = guard(
        source
            .parent()
            .ok_or_else(|| FusionError::new(ErrorCode::InvalidPath, "Source has no parent."))?,
    )?;
    let mut source_file = open_read(source)?;
    let parent = destination
        .parent()
        .ok_or_else(|| FusionError::new(ErrorCode::InvalidPath, "Destination has no parent."))?;
    let _parent = ensure_directory(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        check_metadata(&metadata, destination)?;
        if !metadata.is_file() {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Atomic copy output is not a regular file.",
            ));
        }
    }
    let temporary = parent.join(unique_name(".copy"));
    let mut output = create_new(&temporary)?;
    let result = std::io::copy(&mut source_file, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|error| io_error("Stage atomic copy", destination, error))
        .and_then(|_| {
            #[cfg(windows)]
            {
                windows::rename_handle(&output, destination, true)
                    .map_err(|error| io_error("Publish atomic copy", destination, error))
            }
            #[cfg(not(windows))]
            {
                fs::rename(&temporary, destination)
                    .map_err(|error| io_error("Publish atomic copy", destination, error))
            }
        });
    if result.is_err() {
        #[cfg(windows)]
        let _ = windows::delete_handle(&output);
        #[cfg(not(windows))]
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Unlinks one plain leaf while preserving shared read-only attributes.
pub fn unlink_file(path: &Path) -> Result<(), FusionError> {
    let _guard = guard(
        path.parent()
            .ok_or_else(|| FusionError::new(ErrorCode::InvalidPath, "File has no parent."))?,
    )?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| io_error("Inspect deletion target", path, error))?;
    check_metadata(&metadata, path)?;
    if !metadata.is_file() {
        return Err(FusionError::new(
            ErrorCode::InvalidPath,
            "Unlink requires a plain file.",
        ));
    }
    #[cfg(windows)]
    {
        let file = windows::open_delete(path)?;
        check_metadata(
            &file
                .metadata()
                .map_err(|error| io_error("Inspect deletion handle", path, error))?,
            path,
        )?;
        windows::delete_handle(&file).map_err(|error| io_error("Unlink file", path, error))
    }
    #[cfg(not(windows))]
    {
        fs::remove_file(path).map_err(|error| io_error("Unlink file", path, error))
    }
}

/// Only for newly created, explicitly owned staging trees, never for adoption or
/// snapshot cleanup. Refuses every reparse point, including directory junctions.
pub fn safe_remove_tree(root: &Path) -> Result<(), FusionError> {
    let parent = root.parent().ok_or_else(|| {
        FusionError::new(ErrorCode::InvalidPath, "Cannot remove a filesystem root.")
    })?;
    let _parent = guard(parent)?;
    {
        let _root = guard(root)?;
        for entry in fs::read_dir(root).map_err(|error| io_error("List owned tree", root, error))? {
            let entry = entry.map_err(|error| io_error("Read owned entry", root, error))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| io_error("Inspect owned entry", &path, error))?;
            check_metadata(&metadata, &path)?;
            if metadata.is_dir() {
                safe_remove_tree(&path)?;
            } else {
                unlink_file(&path)?;
            }
        }
    }
    fs::remove_dir(root).map_err(|error| io_error("Remove owned directory", root, error))
}

pub fn list_plain_files(root: &Path) -> Result<Vec<(String, PathBuf)>, FusionError> {
    fn walk(
        root: &Path,
        current: &Path,
        files: &mut Vec<(String, PathBuf)>,
    ) -> Result<(), FusionError> {
        let _guard = guard(current)?;
        for entry in
            fs::read_dir(current).map_err(|error| io_error("List directory", current, error))?
        {
            let path = entry
                .map_err(|error| io_error("Read directory entry", current, error))?
                .path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| io_error("Inspect entry", &path, error))?;
            check_metadata(&metadata, &path)?;
            if metadata.is_dir() {
                walk(root, &path, files)?;
            } else {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|error| io_error("Resolve relative path", &path, error))?;
                let value = relative
                    .to_str()
                    .ok_or_else(|| {
                        FusionError::new(ErrorCode::InvalidPath, "File paths must be Unicode.")
                    })?
                    .replace('\\', "/");
                validate_relative_path(&value)?;
                files.push((value, path));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut unique = std::collections::BTreeSet::new();
    for (relative, _) in &files {
        if !unique.insert(relative.to_lowercase()) {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Case-insensitive file path collision.",
            ));
        }
    }
    Ok(files)
}

pub fn file_identity(path: &Path) -> Result<FileIdentity, FusionError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("Inspect identity", path, error))?;
    check_metadata(&metadata, path)?;
    let _parent = guard(if metadata.is_dir() {
        path
    } else {
        path.parent().ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidPath, "File identity has no parent.")
        })?
    })?;
    #[cfg(windows)]
    {
        windows::identity(path)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(FusionError::new(
            ErrorCode::ProjectionUnavailable,
            "Filesystem identities are unsupported on this platform.",
        ))
    }
}

/// Identity of a retained descriptor without reopening its pathname.
pub(crate) fn identity_from_file(file: &File) -> Result<FileIdentity, FusionError> {
    #[cfg(windows)]
    {
        windows::identity_from_file(file)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file
            .metadata()
            .map_err(|error| FusionError::new(ErrorCode::Io, error.to_string()))?;
        Ok(FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        Err(FusionError::new(
            ErrorCode::ProjectionUnavailable,
            "Descriptor identities are unsupported on this platform.",
        ))
    }
}

/// Newly created staging files must have one physical link before permissions
/// are changed. Published CAS files deliberately allow workspace hardlinks.
pub(crate) fn link_count(file: &File) -> Result<u64, FusionError> {
    #[cfg(windows)]
    {
        windows::link_count(file)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(file
            .metadata()
            .map_err(|error| FusionError::new(ErrorCode::Io, error.to_string()))?
            .nlink())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        Err(FusionError::new(
            ErrorCode::ProjectionUnavailable,
            "Hardlink counts are unsupported on this platform.",
        ))
    }
}

pub fn same_volume(left: &Path, right: &Path) -> Result<bool, FusionError> {
    Ok(file_identity(left)?.volume == file_identity(right)?.volume)
}

/// Publishes the exact verified descriptor opened with DELETE access. This
/// primitive deliberately has no path-based fallback on unsupported systems.
pub fn publish_file_no_replace(file: &File, destination: &Path) -> Result<(), FusionError> {
    let _parent =
        guard(destination.parent().ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidPath, "Destination has no parent.")
        })?)?;
    crate::install_ownership::publish_rename_no_replace(file, destination)
        .map_err(|error| io_error("Publish verified file", destination, error))
}

/// Atomic no-replace directory publication; callers hold the parent guard and
/// per-object lock. Never silently replaces an already published object.
pub(crate) fn publish_directory(source: &Path, destination: &Path) -> Result<(), FusionError> {
    #[cfg(windows)]
    {
        let file = windows::open_delete(source)?;
        let metadata = file
            .metadata()
            .map_err(|error| io_error("Inspect publication directory", source, error))?;
        check_metadata(&metadata, source)?;
        if !metadata.is_dir() {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "CAS publication source must be a directory.",
            ));
        }
        windows::rename_handle(&file, destination, false)
            .map_err(|error| io_error("Publish CAS directory", destination, error))
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renameat2(
                oldfd: i32,
                old: *const std::ffi::c_char,
                newfd: i32,
                new: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let old = std::ffi::CString::new(source.as_os_str().as_bytes())
            .map_err(|error| io_error("Encode source", source, error))?;
        let new = std::ffi::CString::new(destination.as_os_str().as_bytes())
            .map_err(|error| io_error("Encode destination", destination, error))?;
        // SAFETY: nul-terminated paths stay live; RENAME_NOREPLACE is atomic.
        if unsafe { renameat2(-100, old.as_ptr(), -100, new.as_ptr(), 1) } != 0 {
            return Err(io_error(
                "Publish CAS directory",
                destination,
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (source, destination);
        Err(FusionError::new(
            ErrorCode::ProjectionUnavailable,
            "Atomic no-replace directory publication is unsupported on this platform.",
        ))
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    #[derive(Default)]
    struct Information {
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
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information)
            -> i32;
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    pub(super) fn open_delete(path: &Path) -> Result<File, FusionError> {
        OpenOptions::new()
            .read(true)
            .access_mode(0x80000000 | 0x10000)
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)
            .map_err(|error| io_error("Open retained filesystem handle", path, error))
    }

    pub(super) fn identity(path: &Path) -> Result<FileIdentity, FusionError> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)
            .map_err(|error| io_error("Open identity", path, error))?;
        identity_from_file(&file)
    }

    pub(super) fn identity_from_file(file: &File) -> Result<FileIdentity, FusionError> {
        let mut info = Information::default();
        // SAFETY: correctly laid out output and a valid retained handle.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io_error(
                "Read identity",
                Path::new("<retained descriptor>"),
                std::io::Error::last_os_error(),
            ));
        }
        if info.attributes & 0x400 != 0 {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Identity belongs to a reparse point.",
            ));
        }
        Ok(FileIdentity {
            volume: info.volume as u64,
            index: ((info.index_high as u64) << 32) | info.index_low as u64,
        })
    }

    pub(super) fn link_count(file: &File) -> Result<u64, FusionError> {
        let mut info = Information::default();
        // SAFETY: correctly laid out output and a valid retained handle.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(FusionError::new(
                ErrorCode::Io,
                std::io::Error::last_os_error().to_string(),
            ));
        }
        if info.attributes & 0x400 != 0 {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Hardlink count belongs to a reparse point.",
            ));
        }
        Ok(info.links.into())
    }

    pub(super) fn delete_handle(file: &File) -> std::io::Result<()> {
        let flags: u32 = 1 | 2 | 16; // DELETE | POSIX_SEMANTICS | IGNORE_READONLY_ATTRIBUTE
                                     // SAFETY: class 21 expects FILE_DISPOSITION_INFO_EX, a single DWORD.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                21,
                std::ptr::from_ref(&flags).cast(),
                4,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn rename_handle(
        file: &File,
        destination: &Path,
        replace: bool,
    ) -> std::io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        let destination = destination
            .parent()
            .ok_or_else(|| std::io::Error::other("Destination has no parent."))?
            .canonicalize()?
            .join(
                destination
                    .file_name()
                    .ok_or_else(|| std::io::Error::other("Destination has no name."))?,
            );
        let name = destination.as_os_str().encode_wide().collect::<Vec<_>>();
        #[repr(C)]
        struct Rename {
            replace: u32,
            root: *mut std::ffi::c_void,
            length: u32,
            name: [u16; 0],
        }
        let offset = std::mem::offset_of!(Rename, name);
        let name_bytes = name
            .len()
            .checked_mul(2)
            .ok_or_else(|| std::io::Error::other("Path too long."))?;
        let length = offset
            .checked_add(name_bytes)
            .and_then(|length| length.checked_add(std::mem::size_of::<u16>()))
            .ok_or_else(|| std::io::Error::other("Path too long."))?;
        let size = u32::try_from(length).map_err(|_| std::io::Error::other("Path too long."))?;
        let name_bytes =
            u32::try_from(name_bytes).map_err(|_| std::io::Error::other("Path too long."))?;
        let mut buffer = vec![0usize; length.div_ceil(std::mem::size_of::<usize>())];
        let info = buffer.as_mut_ptr().cast::<Rename>();
        // SAFETY: allocation is aligned for HANDLE and covers header and UTF-16
        // name AND its NUL. Win32's path conversion may read that terminator even
        // though FileNameLength excludes it. Allocation padding is not a NUL when
        // the name happens to end exactly on a usize boundary.
        let success = unsafe {
            (*info).replace = if replace { 1 | 2 | 0x40 } else { 0 };
            (*info).root = std::ptr::null_mut();
            (*info).length = name_bytes;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                buffer.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
                name.len(),
            );
            buffer
                .as_mut_ptr()
                .cast::<u8>()
                .add(offset)
                .cast::<u16>()
                .add(name.len())
                .write(0);
            SetFileInformationByHandle(
                file.as_raw_handle(),
                if replace { 22 } else { 3 },
                info.cast(),
                size,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn atomic_rename_terminates_all_utf16_name_alignments_and_long_unicode_paths() {
        use std::collections::BTreeSet;
        use std::os::windows::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        let long_parent = directory
            .path()
            .join("长目录".repeat(20))
            .join("длинный-каталог".repeat(4))
            .join("nested".repeat(12));
        fs::create_dir_all(&long_parent).unwrap();
        let parents = [directory.path().to_owned(), long_parent];
        let mut alignments = BTreeSet::new();
        for parent in parents {
            let mut expected = BTreeSet::new();
            for stem in ["snapshot", "снимок", "🔒-文件"] {
                for suffix in 0..8 {
                    let filename = format!("{stem}{}.json", "x".repeat(suffix));
                    let target = parent.join(&filename);
                    let canonical_target = parent.canonicalize().unwrap().join(&filename);
                    alignments.insert(canonical_target.as_os_str().encode_wide().count() % 4);
                    atomic_write(&target, b"initial exact destination").unwrap();
                    assert_eq!(fs::read(&target).unwrap(), b"initial exact destination");
                    atomic_write(&target, b"replacement exact destination").unwrap();
                    assert_eq!(fs::read(&target).unwrap(), b"replacement exact destination");
                    expected.insert(filename);
                }
            }
            let actual = fs::read_dir(&parent)
                .unwrap()
                .filter_map(|entry| {
                    let entry = entry.unwrap();
                    entry
                        .file_type()
                        .unwrap()
                        .is_file()
                        .then(|| entry.file_name().into_string().unwrap())
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(
                actual, expected,
                "No over-read characters or staging files may survive publication."
            );
        }
        assert_eq!(alignments, BTreeSet::from([0, 1, 2, 3]));
    }

    #[test]
    fn atomic_replacement_and_copy_do_not_mutate_source() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        let output = directory.path().join("nested/copy");
        fs::write(&original, b"original").unwrap();
        copy_file(&original, &output).unwrap();
        atomic_write(&output, b"updated").unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"original");
        assert_eq!(fs::read(&output).unwrap(), b"updated");
        assert!(!original.metadata().unwrap().permissions().readonly());
        assert!(copy_file(&original, &output).is_err());
    }

    #[test]
    fn readonly_hardlink_unlink_never_clears_shared_attribute() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        let linked = directory.path().join("linked");
        fs::write(&original, b"immutable").unwrap();
        fs::hard_link(&original, &linked).unwrap();
        let mut permissions = original.metadata().unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&original, permissions).unwrap();
        unlink_file(&linked).unwrap();
        assert!(!linked.exists());
        assert!(original.metadata().unwrap().permissions().readonly());
        assert_eq!(fs::read(&original).unwrap(), b"immutable");
        unlink_file(&original).unwrap();
    }

    #[test]
    fn replacing_readonly_hardlink_keeps_original_readonly_and_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("immutable");
        let linked = directory.path().join("workspace-file");
        fs::write(&original, b"original").unwrap();
        fs::hard_link(&original, &linked).unwrap();
        let mut permissions = original.metadata().unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&original, permissions).unwrap();
        atomic_write(&linked, b"separate mutable output").unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"original");
        assert!(original.metadata().unwrap().permissions().readonly());
        assert_eq!(fs::read(&linked).unwrap(), b"separate mutable output");
        assert!(!linked.metadata().unwrap().permissions().readonly());
        assert_ne!(
            file_identity(&original).unwrap(),
            file_identity(&linked).unwrap()
        );
        unlink_file(&original).unwrap();
    }

    #[test]
    fn cleanup_rejects_symlink_and_preserves_outside_sentinel() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("owned");
        let outside = directory.path().join("outside");
        fs::create_dir(&tree).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"safe").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, tree.join("escape")).unwrap();
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_dir(&outside, tree.join("escape")).is_err() {
                // Creating symlinks requires Developer Mode or elevation. The
                // traversal test remains covered without that optional right.
                assert!(validate_relative_path("../outside").is_err());
                return;
            }
        }
        assert!(safe_remove_tree(&tree).is_err());
        assert!(list_plain_files(&tree).is_err());
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"safe");
    }
}
