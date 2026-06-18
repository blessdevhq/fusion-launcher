use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

pub(crate) fn extract_archive_safely(
    archive_path: &Path,
    install_dir: &Path,
) -> Result<(), String> {
    let extension = archive_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "zip" => extract_zip_safely(archive_path, install_dir),
        "7z" => extract_7z_safely(archive_path, install_dir),
        _ => Err(format!("unsupported_emulator_archive:{extension}")),
    }
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

fn extract_7z_safely(archive_path: &Path, install_dir: &Path) -> Result<(), String> {
    let install_dir = install_dir.to_path_buf();
    let callback_install_dir = install_dir.clone();
    sevenz_rust::decompress_file_with_extract_fn(
        archive_path,
        &install_dir,
        move |entry, reader, _| {
            let relative = safe_relative_path(entry.name()).map_err(sevenz_rust::Error::other)?;
            let output_path = callback_install_dir.join(relative);
            if entry.is_directory() {
                fs::create_dir_all(&output_path).map_err(sevenz_rust::Error::io)?;
                return Ok(true);
            }
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(sevenz_rust::Error::io)?;
            }
            let mut output = File::create(&output_path).map_err(sevenz_rust::Error::io)?;
            io::copy(reader, &mut output).map_err(sevenz_rust::Error::io)?;
            Ok(true)
        },
    )
    .map_err(|error| format!("Failed to extract 7z emulator archive: {error}"))
}

fn extract_zip_safely(archive_path: &Path, install_dir: &Path) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|error| format!("Failed to open emulator zip archive: {error}"))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("Failed to read emulator zip archive: {error}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read zip entry: {error}"))?;
        let relative = safe_relative_path(entry.name())?;
        let output_path = install_dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output_path)
                .map_err(|error| format!("Failed to create emulator folder: {error}"))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create emulator folder: {error}"))?;
        }
        let mut output = File::create(&output_path)
            .map_err(|error| format!("Failed to create emulator file: {error}"))?;
        io::copy(&mut entry, &mut output)
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

        let error = extract_zip_safely(&archive_path, temp.path()).unwrap_err();

        assert!(error.contains("path traversal"));
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
