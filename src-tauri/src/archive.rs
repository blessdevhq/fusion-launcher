use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

pub(crate) fn extract_archive_safely(
    archive_path: &Path,
    install_dir: &Path,
) -> Result<(), String> {
    extract_archive_with_cancel(archive_path, install_dir, &|| false)
}

pub(crate) fn extract_archive_safely_guarded(
    archive_path: &Path,
    install_dir: &Path,
    lease: crate::download_operations::OperationLease,
) -> Result<(), String> {
    let _ownership_guard = lease.protect_writes()?;
    extract_archive_with_cancel(archive_path, install_dir, &|| lease.is_cancelled())
}

fn check_extract_cancelled(is_cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if is_cancelled() {
        Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

fn copy_extract_with_cancel(
    reader: &mut (impl io::Read + ?Sized),
    writer: &mut impl io::Write,
    is_cancelled: &impl Fn() -> bool,
) -> io::Result<()> {
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                crate::downloads::DOWNLOAD_CANCELLED_MESSAGE,
            ));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                crate::downloads::DOWNLOAD_CANCELLED_MESSAGE,
            ));
        }
        writer.write_all(&buffer[..read])?;
    }
}

fn extract_archive_with_cancel(
    archive_path: &Path,
    install_dir: &Path,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    check_extract_cancelled(is_cancelled)?;
    let _root_guard = ensure_plain_archive_directory(install_dir)?;
    let extension = archive_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "zip" => extract_zip_safely(archive_path, install_dir, is_cancelled),
        "7z" => extract_7z_safely(archive_path, install_dir, is_cancelled),
        _ => Err(format!("unsupported_emulator_archive:{extension}")),
    }
}

fn ensure_plain_archive_directory(
    path: &Path,
) -> Result<crate::install_ownership::DirectoryProtection, String> {
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(existing.to_path_buf());
                existing = existing
                    .parent()
                    .ok_or("Archive output has no existing parent.")?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    // Retain each verified ancestor while creating and locking its child.
    // A junction cannot be swapped between validation and the output open.
    let mut guard = crate::install_ownership::lock_plain_directory(existing)?;
    for directory in missing.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        let child = crate::install_ownership::lock_plain_directory(&directory)?;
        guard = child;
    }
    Ok(guard)
}

pub(crate) fn resolve_executable(
    install_dir: &Path,
    exe_relative_path: &str,
    display_name: &str,
) -> Result<PathBuf, String> {
    let relative = safe_relative_path(exe_relative_path)?;
    let direct = install_dir.join(relative);
    let executable = if direct.is_file() {
        direct
    } else {
        find_file_by_name(
            install_dir,
            Path::new(exe_relative_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(exe_relative_path),
        )
        .ok_or_else(|| {
            format!("{exe_relative_path} was not found after installing {display_name}.")
        })?
    };
    let canonical_root = fs::canonicalize(install_dir)
        .map_err(|error| format!("Failed to inspect emulator install folder: {error}"))?;
    let canonical_executable = fs::canonicalize(&executable)
        .map_err(|error| format!("Failed to inspect emulator executable: {error}"))?;
    if !canonical_executable.starts_with(canonical_root) {
        return Err("Installed emulator executable escaped the install folder.".into());
    }
    Ok(executable)
}

pub(crate) fn reset_staging_dir(install_root: &Path, staging_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(install_root)
        .map_err(|error| format!("Failed to create emulator install folder: {error}"))?;
    remove_dir_inside(install_root, staging_dir)?;
    fs::create_dir_all(staging_dir)
        .map_err(|error| format!("Failed to create emulator staging folder: {error}"))
}

pub(crate) fn replace_directory(
    install_root: &Path,
    install_dir: &Path,
    staging_dir: &Path,
) -> Result<(), String> {
    remove_dir_inside(install_root, install_dir)?;
    fs::rename(staging_dir, install_dir)
        .map_err(|error| format!("Failed to finalize emulator install: {error}"))
}

fn extract_7z_safely(
    archive_path: &Path,
    install_dir: &Path,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let install_dir = install_dir.to_path_buf();
    let callback_install_dir = install_dir.clone();
    sevenz_rust::decompress_file_with_extract_fn(
        archive_path,
        &install_dir,
        move |entry, reader, _| {
            check_extract_cancelled(is_cancelled).map_err(sevenz_rust::Error::other)?;
            let relative = safe_relative_path(entry.name()).map_err(sevenz_rust::Error::other)?;
            let output_path = callback_install_dir.join(relative);
            if entry.is_directory() {
                ensure_plain_archive_directory(&output_path).map_err(sevenz_rust::Error::other)?;
                return Ok(true);
            }
            let _parent_guard = output_path
                .parent()
                .map(ensure_plain_archive_directory)
                .transpose()
                .map_err(sevenz_rust::Error::other)?;
            let mut output = File::options()
                .write(true)
                .create_new(true)
                .open(&output_path)
                .map_err(sevenz_rust::Error::io)?;
            copy_extract_with_cancel(reader, &mut output, is_cancelled)
                .map_err(sevenz_rust::Error::io)?;
            Ok(true)
        },
    )
    .map_err(|error| format!("Failed to extract 7z emulator archive: {error}"))
}

fn extract_zip_safely(
    archive_path: &Path,
    install_dir: &Path,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|error| format!("Failed to open emulator zip archive: {error}"))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("Failed to read emulator zip archive: {error}"))?;
    for index in 0..archive.len() {
        check_extract_cancelled(is_cancelled)?;
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read zip entry: {error}"))?;
        let relative = safe_relative_path(entry.name())?;
        let output_path = install_dir.join(relative);
        if entry.is_dir() {
            ensure_plain_archive_directory(&output_path)?;
            continue;
        }
        let _parent_guard = output_path
            .parent()
            .map(ensure_plain_archive_directory)
            .transpose()?;
        let mut output = File::options()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|error| format!("Failed to create emulator file: {error}"))?;
        copy_extract_with_cancel(&mut entry, &mut output, is_cancelled)
            .map_err(|error| format!("Failed to extract emulator file: {error}"))?;
    }
    Ok(())
}

fn safe_relative_path(input: &str) -> Result<PathBuf, String> {
    let path = Path::new(input.trim());
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("Emulator archive contains an unsafe path.".to_string());
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => safe.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Emulator archive contains a path traversal entry.".to_string())
            }
        }
    }
    if safe.as_os_str().is_empty() {
        Err("Emulator archive contains an empty path.".to_string())
    } else {
        Ok(safe)
    }
}

fn find_file_by_name(root: &Path, file_name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_by_name(&path, file_name) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(file_name))
            .unwrap_or(false)
        {
            return Some(path);
        }
    }
    None
}

fn remove_dir_inside(root: &Path, target: &Path) -> Result<(), String> {
    remove_directory_inside(root, target).map(|_| ())
}

pub(crate) fn remove_directory_inside(root: &Path, target: &Path) -> Result<bool, String> {
    if !target.exists() {
        return Ok(false);
    }
    let root = fs::canonicalize(root)
        .map_err(|error| format!("Failed to inspect emulator root: {error}"))?;
    let target = fs::canonicalize(target)
        .map_err(|error| format!("Failed to inspect emulator folder: {error}"))?;
    if target == root || !target.starts_with(&root) {
        return Err(format!(
            "Refusing to remove emulator folder outside app data: {}",
            target.display()
        ));
    }
    fs::remove_dir_all(target)
        .map_err(|error| format!("Failed to remove emulator folder: {error}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::*;

    #[test]
    fn zip_extraction_rejects_path_traversal() {
        let temp = tempdir().unwrap();
        let archive_path = temp.path().join("bad.zip");
        let file = File::create(&archive_path).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("../outside.exe", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"bad").unwrap();
        writer.finish().unwrap();

        let error = extract_archive_safely(&archive_path, temp.path()).unwrap_err();

        assert!(error.contains("path traversal"));
    }

    #[test]
    fn zip_extraction_preserves_preexisting_output() {
        let temp = tempdir().unwrap();
        let archive_path = temp.path().join("input.zip");
        let target = temp.path().join("system.bin");
        fs::write(&target, b"user sentinel").unwrap();
        let mut writer = ZipWriter::new(File::create(&archive_path).unwrap());
        writer
            .start_file("system.bin", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"archive replacement").unwrap();
        writer.finish().unwrap();
        assert!(extract_archive_safely(&archive_path, temp.path()).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"user sentinel");
    }

    #[test]
    #[cfg(windows)]
    fn zip_extraction_refuses_junction_ancestor_and_preserves_target() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("output");
        let external = temp.path().join("external");
        fs::create_dir(&output).unwrap();
        fs::create_dir(&external).unwrap();
        fs::write(external.join("system.bin"), b"user sentinel").unwrap();
        let junction = output.join("linked");
        let command = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &junction.to_string_lossy(),
                &external.to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(command.status.success(), "junction creation failed");
        let archive_path = temp.path().join("input.zip");
        let mut writer = ZipWriter::new(File::create(&archive_path).unwrap());
        writer
            .start_file("linked/system.bin", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"archive replacement").unwrap();
        writer.finish().unwrap();
        let result = extract_archive_safely(&archive_path, &output);
        fs::remove_dir(&junction).unwrap();
        assert!(result.unwrap_err().contains("reparse"));
        assert_eq!(
            fs::read(external.join("system.bin")).unwrap(),
            b"user sentinel"
        );
    }

    #[test]
    fn archive_copy_observes_cancel_between_read_and_write() {
        use std::cell::Cell;
        struct CancelOnRead<'a>(&'a Cell<bool>);
        impl io::Read for CancelOnRead<'_> {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                output[0] = 1;
                self.0.set(true);
                Ok(1)
            }
        }
        let cancelled = Cell::new(false);
        let mut output = Vec::new();
        let error = copy_extract_with_cancel(&mut CancelOnRead(&cancelled), &mut output, &|| {
            cancelled.get()
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(output.is_empty());
    }

    #[test]
    fn cancelled_archive_extraction_does_not_create_payload() {
        let temp = tempdir().unwrap();
        let registry = crate::download_operations::OperationCoordinator::new();
        let lease = registry.start("asset").unwrap();
        registry.cancel("asset").unwrap();
        let error =
            extract_archive_safely_guarded(&temp.path().join("not-opened.zip"), temp.path(), lease)
                .unwrap_err();
        assert_eq!(error, crate::downloads::DOWNLOAD_CANCELLED_MESSAGE);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn executable_can_be_found_in_nested_archive_folder() {
        let temp = tempdir().unwrap();
        let nested = temp.path().join("Mesen");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("Mesen.exe"), b"exe").unwrap();

        let executable = resolve_executable(temp.path(), "Mesen.exe", "Mesen2").unwrap();

        assert!(executable.ends_with("Mesen.exe"));
    }

    #[test]
    fn managed_directory_removal_refuses_outside_target() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("Emulators");
        let outside = temp.path().join("Outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let error = remove_directory_inside(&root, &outside).unwrap_err();

        assert!(error.contains("outside app data"));
        assert!(outside.exists());
    }

    #[test]
    fn managed_directory_removal_reports_missing_target() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("Emulators");
        fs::create_dir_all(&root).unwrap();

        let removed = remove_directory_inside(&root, &root.join("nes")).unwrap();

        assert!(!removed);
    }
}
