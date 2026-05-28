use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;
use zip::ZipArchive;

use crate::platform::{default_launch_args_template, MVP_PLATFORM_CONFIGS};
use crate::schema::EmulatorConfig;
use crate::storage::RepositoryStore;
use crate::AppState;

const MESEN_PLATFORM: &str = "nes";
const MESEN_EMULATOR_NAME: &str = "Mesen2";
const MESEN_VERSION: &str = "2.1.1";
const MESEN_DOWNLOAD_URL: &str =
    "https://github.com/SourMesen/Mesen2/releases/download/2.1.1/Mesen_2.1.1_Windows.zip";
const MESEN_SHA256: &str = "23ccc2bc060b663c68dad3a8c5d6da7d23a50f872d04f135bafa2b04ff7d5cbe";
const MESEN_EXECUTABLE: &str = "Mesen.exe";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedEmulator {
    pub platform: String,
    pub platform_label: String,
    pub emulator_name: String,
    pub version: Option<String>,
    pub download_url: Option<String>,
    pub sha256: Option<String>,
    pub executable_name: String,
    pub status: String,
    pub installed_path: Option<String>,
    pub message: Option<String>,
}

struct EmulatorManifest {
    platform: &'static str,
    emulator_name: &'static str,
    version: &'static str,
    download_url: &'static str,
    sha256: &'static str,
    executable_name: &'static str,
}

const MESEN_MANIFEST: EmulatorManifest = EmulatorManifest {
    platform: MESEN_PLATFORM,
    emulator_name: MESEN_EMULATOR_NAME,
    version: MESEN_VERSION,
    download_url: MESEN_DOWNLOAD_URL,
    sha256: MESEN_SHA256,
    executable_name: MESEN_EXECUTABLE,
};

#[tauri::command]
pub fn get_recommended_emulators(
    state: State<'_, AppState>,
) -> Result<Vec<RecommendedEmulator>, String> {
    let store = lock_store(&state)?;
    recommended_emulators(&store)
}

#[tauri::command]
pub async fn install_recommended_emulator(
    platform: String,
    state: State<'_, AppState>,
) -> Result<EmulatorConfig, String> {
    if platform.trim() != MESEN_MANIFEST.platform {
        return Err("Automatic setup is currently available only for NES / Mesen2.".to_string());
    }

    let archive_path = download_archive_to_temp_file(&state.data_dir, &MESEN_MANIFEST).await?;
    let executable_path = install_archive_file(&state.data_dir, &MESEN_MANIFEST, &archive_path)?;
    let _ = fs::remove_file(&archive_path);
    let store = lock_store(&state)?;
    persist_installed_emulator(&store, &MESEN_MANIFEST, &executable_path)
}

fn persist_installed_emulator(
    store: &RepositoryStore,
    manifest: &EmulatorManifest,
    executable_path: &Path,
) -> Result<EmulatorConfig, String> {
    let executable = executable_path.to_string_lossy().to_string();
    let status = validate_emulator_executable(executable_path);
    if status != "valid" {
        return Err(format!(
            "Installed emulator executable is not valid: {}",
            executable_path.display()
        ));
    }

    store.upsert_emulator_config(
        manifest.platform,
        Some(&executable),
        status,
        Some(manifest.version),
        default_launch_args_template(manifest.platform),
    )
}

fn recommended_emulators(store: &RepositoryStore) -> Result<Vec<RecommendedEmulator>, String> {
    let mut items = Vec::new();
    for platform in MVP_PLATFORM_CONFIGS {
        let config = store.get_emulator_config(platform.id)?;
        let installed_path = config.as_ref().and_then(|config| config.exe_path.clone());
        let installed = installed_path
            .as_deref()
            .map(|path| validate_emulator_executable(Path::new(path)) == "valid")
            .unwrap_or(false);

        if platform.id == MESEN_PLATFORM {
            items.push(RecommendedEmulator {
                platform: platform.id.to_string(),
                platform_label: platform.label.to_string(),
                emulator_name: MESEN_EMULATOR_NAME.to_string(),
                version: Some(MESEN_VERSION.to_string()),
                download_url: Some(MESEN_DOWNLOAD_URL.to_string()),
                sha256: Some(MESEN_SHA256.to_string()),
                executable_name: MESEN_EXECUTABLE.to_string(),
                status: if installed { "installed" } else { "available" }.to_string(),
                installed_path,
                message: Some(if installed {
                    "Mesen2 is ready for the built-in NES demo.".to_string()
                } else {
                    "RetroHydra can install pinned Mesen2 for the built-in NES demo.".to_string()
                }),
            });
        } else {
            items.push(RecommendedEmulator {
                platform: platform.id.to_string(),
                platform_label: platform.label.to_string(),
                emulator_name: platform.emulator_name.to_string(),
                version: None,
                download_url: None,
                sha256: None,
                executable_name: platform.executable_hint.to_string(),
                status: if installed { "installed" } else { "manual" }.to_string(),
                installed_path,
                message: Some(if installed {
                    format!("{} is configured.", platform.emulator_name)
                } else {
                    format!("Manual setup required for {}.", platform.emulator_name)
                }),
            });
        }
    }
    Ok(items)
}

async fn download_archive_to_temp_file(
    data_dir: &Path,
    manifest: &EmulatorManifest,
) -> Result<PathBuf, String> {
    let temp_dir = data_dir.join("Temp").join("emulators");
    fs::create_dir_all(&temp_dir)
        .map_err(|error| format!("Failed to create emulator temp folder: {error}"))?;
    let archive_path = temp_dir.join(format!(
        "{}-{}.zip",
        safe_segment(manifest.emulator_name),
        safe_segment(manifest.version)
    ));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Failed to initialize emulator downloader: {error}"))?;
    let bytes = client
        .get(manifest.download_url)
        .send()
        .await
        .map_err(|error| format!("Failed to download {}: {error}", manifest.emulator_name))?
        .error_for_status()
        .map_err(|error| format!("Emulator download returned an error: {error}"))?
        .bytes()
        .await
        .map_err(|error| format!("Failed to read emulator download: {error}"))?;

    let actual = sha256_hex(&bytes);
    if !actual.eq_ignore_ascii_case(manifest.sha256) {
        return Err(format!(
            "Emulator archive SHA-256 mismatch: expected {}, got {}",
            manifest.sha256, actual
        ));
    }

    fs::write(&archive_path, &bytes)
        .map_err(|error| format!("Failed to save emulator archive: {error}"))?;
    Ok(archive_path)
}

fn install_archive_file(
    data_dir: &Path,
    manifest: &EmulatorManifest,
    archive_path: &Path,
) -> Result<PathBuf, String> {
    let archive_hash = crate::downloads::hash_file(archive_path)?;
    if !archive_hash.eq_ignore_ascii_case(manifest.sha256) {
        return Err(format!(
            "Emulator archive SHA-256 mismatch: expected {}, got {}",
            manifest.sha256, archive_hash
        ));
    }

    let bytes = fs::read(archive_path)
        .map_err(|error| format!("Failed to read emulator archive: {error}"))?;
    install_archive_bytes(data_dir, manifest, &bytes)
}

fn install_archive_bytes(
    data_dir: &Path,
    manifest: &EmulatorManifest,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let actual = sha256_hex(bytes);
    if !actual.eq_ignore_ascii_case(manifest.sha256) {
        return Err(format!(
            "Emulator archive SHA-256 mismatch: expected {}, got {}",
            manifest.sha256, actual
        ));
    }

    let emulator_root = data_dir.join("Emulators");
    let install_dir = emulator_root.join(format!(
        "{}-{}",
        safe_segment(manifest.emulator_name),
        safe_segment(manifest.version)
    ));
    reset_install_dir(&emulator_root, &install_dir)?;

    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|error| format!("Failed to read emulator zip archive: {error}"))?;
    extract_zip_safely(&mut archive, &install_dir)?;

    find_executable(&install_dir, manifest.executable_name).ok_or_else(|| {
        format!(
            "{} was not found after installing {}.",
            manifest.executable_name, manifest.emulator_name
        )
    })
}

fn reset_install_dir(emulator_root: &Path, install_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(emulator_root)
        .map_err(|error| format!("Failed to create emulator folder: {error}"))?;
    if install_dir.exists() {
        let root = fs::canonicalize(emulator_root)
            .map_err(|error| format!("Failed to inspect emulator folder: {error}"))?;
        let target = fs::canonicalize(install_dir)
            .map_err(|error| format!("Failed to inspect install folder: {error}"))?;
        if !target.starts_with(root) {
            return Err(format!(
                "Refusing to remove emulator folder outside app data: {}",
                target.display()
            ));
        }
        fs::remove_dir_all(install_dir)
            .map_err(|error| format!("Failed to replace emulator install folder: {error}"))?;
    }
    fs::create_dir_all(install_dir)
        .map_err(|error| format!("Failed to create emulator install folder: {error}"))?;
    Ok(())
}

fn extract_zip_safely<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    install_dir: &Path,
) -> Result<(), String> {
    let mut created_paths = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read zip entry: {error}"))?;
        let relative = safe_zip_entry_path(entry.name())?;
        let output_path = install_dir.join(relative);
        if !created_paths.insert(output_path.clone()) {
            continue;
        }

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

fn safe_zip_entry_path(name: &str) -> Result<PathBuf, String> {
    if name.trim().is_empty() || name.contains('\\') {
        return Err("Emulator archive contains an unsafe path.".to_string());
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return Err("Emulator archive contains an absolute path.".to_string());
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

fn find_executable(root: &Path, executable_name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_executable(&path, executable_name) {
                return Some(found);
            }
            continue;
        }
        if path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .map(|file_name| file_name.eq_ignore_ascii_case(executable_name))
            .unwrap_or(false)
        {
            return Some(path);
        }
    }
    None
}

fn validate_emulator_executable(path: &Path) -> &'static str {
    if !path.exists() {
        return "missing";
    }
    if !path.is_file() {
        return "invalid";
    }
    if cfg!(windows)
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| !extension.eq_ignore_ascii_case("exe"))
            .unwrap_or(true)
    {
        return "invalid";
    }
    "valid"
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn safe_segment(value: &str) -> String {
    crate::downloads::safe_segment(value)
}

fn lock_store<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, RepositoryStore>, String> {
    state
        .store
        .lock()
        .map_err(|_| "Repository store lock is poisoned.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn test_manifest(expected_sha256: String) -> EmulatorManifest {
        EmulatorManifest {
            platform: MESEN_PLATFORM,
            emulator_name: MESEN_EMULATOR_NAME,
            version: MESEN_VERSION,
            download_url: MESEN_DOWNLOAD_URL,
            sha256: Box::leak(expected_sha256.into_boxed_str()),
            executable_name: MESEN_EXECUTABLE,
        }
    }

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();
        for (path, bytes) in entries {
            writer.start_file(*path, options).unwrap();
            io::Write::write_all(&mut writer, bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn pinned_mesen_metadata_matches_verified_hash() {
        assert_eq!(MESEN_MANIFEST.platform, "nes");
        assert_eq!(MESEN_MANIFEST.version, "2.1.1");
        assert_eq!(
            MESEN_MANIFEST.sha256,
            "23ccc2bc060b663c68dad3a8c5d6da7d23a50f872d04f135bafa2b04ff7d5cbe"
        );
        assert!(MESEN_MANIFEST
            .download_url
            .ends_with("Mesen_2.1.1_Windows.zip"));
    }

    #[test]
    fn hash_mismatch_rejects_install() {
        let dir = tempdir().unwrap();
        let bytes = zip_bytes(&[("Mesen.exe", b"fake")]);
        let manifest = test_manifest("0".repeat(64));

        let error = install_archive_bytes(dir.path(), &manifest, &bytes).unwrap_err();

        assert!(error.contains("SHA-256 mismatch"));
    }

    #[test]
    fn unsafe_zip_paths_are_rejected() {
        let dir = tempdir().unwrap();
        let bytes = zip_bytes(&[("../Mesen.exe", b"fake")]);
        let manifest = test_manifest(sha256_hex(&bytes));

        let error = install_archive_bytes(dir.path(), &manifest, &bytes).unwrap_err();

        assert!(error.contains("path traversal"));
    }

    #[test]
    fn successful_install_extracts_mesen_executable() {
        let dir = tempdir().unwrap();
        let bytes = zip_bytes(&[("Mesen/Mesen.exe", b"fake exe")]);
        let manifest = test_manifest(sha256_hex(&bytes));

        let executable = install_archive_bytes(dir.path(), &manifest, &bytes).unwrap();

        assert!(executable.ends_with("Mesen.exe"));
        assert!(executable.exists());
    }

    #[test]
    fn successful_install_saves_valid_emulator_config() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();
        let executable = dir.path().join("Mesen.exe");
        fs::write(&executable, b"fake exe").unwrap();

        let config = persist_installed_emulator(&store, &MESEN_MANIFEST, &executable).unwrap();

        assert_eq!(config.platform, "nes");
        assert_eq!(config.status, "valid");
        assert_eq!(config.version.as_deref(), Some("2.1.1"));
        assert_eq!(
            config.launch_args_template.as_deref(),
            Some(default_launch_args_template("nes").unwrap())
        );
    }

    #[test]
    fn unsupported_platforms_are_listed_as_manual() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();

        let items = recommended_emulators(&store).unwrap();
        let ps1 = items.iter().find(|item| item.platform == "ps1").unwrap();
        let nes = items.iter().find(|item| item.platform == "nes").unwrap();

        assert_eq!(ps1.status, "manual");
        assert_eq!(nes.status, "available");
    }
}
