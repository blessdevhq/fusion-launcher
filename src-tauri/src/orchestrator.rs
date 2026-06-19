use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::archive;
use crate::commands;
use crate::emulator_profiles::{
    load_emulator_profile, EmulatorInstallResult, EmulatorProfile, EmulatorStatus,
    InstallProgressEvent, InstallResult, VersionStrategy,
};
use crate::github_resolver::{resolve_github_latest, ResolvedAsset};
use crate::manifest::Manifest;
use crate::schema::SourceUri;
use crate::security::validate_repository_url;
use crate::storage::RepositoryStore;
use crate::AppState;

const RESOLVED_ASSET_CACHE_HOURS: i64 = 24;
const MAX_EMULATOR_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
struct CachedResolvedAsset {
    resolved_at: DateTime<Utc>,
    asset: ResolvedAsset,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallGameOptions {
    pub game_target_dir: Option<String>,
    pub emulator_target_dir: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ResolvedInstallGameOptions {
    game_target_dir: Option<PathBuf>,
    emulator_target_dir: Option<PathBuf>,
}

#[tauri::command]
pub async fn install_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    options: Option<InstallGameOptions>,
) -> Result<InstallResult, String> {
    let options = match resolve_install_game_options(options) {
        Ok(options) => options,
        Err(error) => {
            return Ok(InstallResult {
                game_id,
                status: "error".to_string(),
                error_code: Some(error_code(&error)),
                message: Some(error),
            });
        }
    };
    match install_game_inner(&app, &state, &game_id, &options).await {
        Ok(result) => Ok(result),
        Err(error) => Ok(InstallResult {
            game_id,
            status: "error".to_string(),
            error_code: Some(error_code(&error)),
            message: Some(error),
        }),
    }
}

/// Fetch a manifest from `url`, import its catalog into the store, and install
/// the game identified by `title_id` through the normal pipeline.
///
/// This is the manifest-driven entry point: the magnet ROM is handed to the
/// torrent engine, the emulator is installed over HTTP (from the manifest's
/// `core_bundle_url` when present, otherwise the platform profile), and on
/// success the game reaches "Ready to play" so a later `launch_game` resolves
/// the emulator `.exe` and the downloaded ROM.
#[tauri::command]
pub async fn install_game_from_manifest(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    title_id: String,
) -> Result<InstallResult, String> {
    let manifest = crate::manifest::fetch_manifest_inner(&url)
        .await
        .map_err(|error| error.to_string())?;
    let source_url = crate::manifest::manifest_source_url(&url, &manifest);

    match install_game_from_manifest_inner(&app, &state, &manifest, &source_url, &title_id).await {
        Ok(result) => Ok(result),
        Err(error) => Ok(InstallResult {
            game_id: title_id,
            status: "error".to_string(),
            error_code: Some(error_code(&error)),
            message: Some(error),
        }),
    }
}

/// Register a manifest as a library source WITHOUT installing any game. The
/// manifest's catalog becomes browsable (its games show as available-to-install)
/// and each game's emulator bundle is persisted so a later on-demand install
/// pulls it in exactly as a direct manifest install would. First-run onboarding
/// uses this: add a source, then install games whenever the user wants — nothing
/// is downloaded here.
#[tauri::command]
pub async fn add_manifest_source(
    state: State<'_, AppState>,
    url: String,
) -> Result<crate::schema::RepositorySummary, String> {
    add_manifest_source_inner(&state, &url).await
}

/// Reusable body of [`add_manifest_source`], also called by the unified
/// `add_source` command after it detects the input is a manifest.
pub(crate) async fn add_manifest_source_inner(
    state: &AppState,
    url: &str,
) -> Result<crate::schema::RepositorySummary, String> {
    let manifest = crate::manifest::fetch_manifest_inner(url)
        .await
        .map_err(|error| error.to_string())?;
    let source_url = crate::manifest::manifest_source_url(url, &manifest);
    let schema = manifest.to_repository_schema();

    let summary = {
        let mut store = lock_store(state)?;
        store.store_repository(&source_url, &schema)?
    };

    // Persist every game's emulator bundle so installing it later from the
    // library pulls the manifest's core_bundle_url, just like a direct install.
    for game in &manifest.games {
        let Some(bundle_url) = game.assets.core_bundle_url.as_deref() else {
            continue;
        };
        let bundle_url = bundle_url.trim();
        if bundle_url.is_empty() {
            continue;
        }
        let stored_game_id = crate::security::global_id(&schema.metadata.id, &game.title_id);
        persist_manifest_emulator_bundle(
            state,
            &stored_game_id,
            &PersistedEmulatorBundle {
                url: bundle_url.to_string(),
                sha256: game.assets.core_bundle_sha256.clone(),
                asset_id: Some(crate::security::global_id(
                    &schema.metadata.id,
                    &game.emulator_asset_id(),
                )),
                executable: non_empty_executable(&game.launch_config.executable),
            },
        )?;
    }

    crate::logging::log_event(
        &state.data_dir,
        "manifest_source_added",
        &[("repository_id", summary.id.as_str())],
    );

    Ok(summary)
}

/// Import a parsed [`Manifest`] into the store and install one of its games.
/// Separated from the command so it can be unit-tested and reused with an
/// already-fetched manifest.
pub(crate) async fn install_game_from_manifest_inner(
    app: &AppHandle,
    state: &AppState,
    manifest: &Manifest,
    source_url: &str,
    title_id: &str,
) -> Result<InstallResult, String> {
    if !manifest.games.iter().any(|game| game.title_id == title_id) {
        return Err(format!("unknown_manifest_game:{title_id}"));
    }

    let schema = manifest.to_repository_schema();
    let stored_game_id = crate::security::global_id(&schema.metadata.id, title_id);
    {
        let mut store = lock_store(state)?;
        store.store_repository(source_url, &schema)?;
    }

    // Persist the manifest's emulator bundle (if any) with the game. The actual
    // download/extraction and the keys check happen inside install_game_inner,
    // so the bundle's own keys/BIOS are fetched before they are validated and a
    // missing local copy never blocks the install up front.
    if let Some(game) = manifest.games.iter().find(|game| game.title_id == title_id) {
        if let Some(url) = game.assets.core_bundle_url.as_deref() {
            let url = url.trim();
            if !url.is_empty() {
                persist_manifest_emulator_bundle(
                    state,
                    &stored_game_id,
                    &PersistedEmulatorBundle {
                        url: url.to_string(),
                        sha256: game.assets.core_bundle_sha256.clone(),
                        asset_id: Some(crate::security::global_id(
                            &schema.metadata.id,
                            &game.emulator_asset_id(),
                        )),
                        executable: non_empty_executable(&game.launch_config.executable),
                    },
                )?;
            }
        }
    }

    install_game_inner(
        app,
        state,
        &stored_game_id,
        &ResolvedInstallGameOptions::default(),
    )
    .await
}

/// Optional emulator download source supplied by a manifest. When present it
/// replaces the platform profile's resolved download URL (and its hash check).
#[derive(Clone)]
pub(crate) struct EmulatorSourceOverride {
    pub url: String,
    pub sha256: Option<String>,
    pub asset_id: Option<String>,
    /// Executable to locate after extraction. Required for platforms without a
    /// bundled profile (Switch), ignored when a profile defines the executable.
    pub executable: Option<String>,
}

/// Emulator bundle persisted alongside a manifest game so the regular install
/// path can fetch it — and the keys/BIOS it carries — at install time instead
/// of requiring the keys to already be on disk.
#[derive(Serialize, Deserialize)]
struct PersistedEmulatorBundle {
    url: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    executable: Option<String>,
}

fn manifest_emulator_config_key(game_id: &str) -> String {
    format!("manifest_emulator:{game_id}")
}

fn persist_manifest_emulator_bundle(
    state: &AppState,
    game_id: &str,
    bundle: &PersistedEmulatorBundle,
) -> Result<(), String> {
    let json = serde_json::to_string(bundle).map_err(|error| error.to_string())?;
    lock_store(state)?
        .set_config(&manifest_emulator_config_key(game_id), &json)
        .map(|_| ())
}

/// Read the persisted emulator bundle for a game, if any, as an install override.
fn manifest_emulator_override(
    state: &AppState,
    game_id: &str,
) -> Result<Option<EmulatorSourceOverride>, String> {
    let Some(raw) = lock_store(state)?.get_config(&manifest_emulator_config_key(game_id))? else {
        return Ok(None);
    };
    let bundle: PersistedEmulatorBundle =
        serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    Ok(Some(EmulatorSourceOverride {
        url: bundle.url,
        sha256: bundle.sha256,
        asset_id: bundle.asset_id,
        executable: bundle.executable,
    }))
}

/// Register any required system files (Switch `prod.keys`, BIOS, …) that shipped
/// inside a freshly-installed emulator archive, so the keys check runs against
/// what the bundle delivered. No-op when the platform has no default profile.
fn adopt_bundled_system_files(
    state: &AppState,
    platform: &str,
    emulator_asset_id: Option<&str>,
) -> Result<(), String> {
    if let Some(profile) = crate::setup_profiles::get_default_platform_setup_profile(platform) {
        let library_root = crate::commands::library_root_for_app_state(state);
        let store = lock_store(state)?;
        let install_dir = emulator_asset_id
            .and_then(|asset_id| {
                store
                    .get_download(asset_id)
                    .ok()
                    .flatten()
                    .filter(|record| matches!(record.status.as_str(), "ready" | "completed"))
                    .and_then(|record| record.local_path)
                    .map(PathBuf::from)
            })
            .and_then(|path| {
                if path.is_dir() {
                    Some(path)
                } else {
                    path.parent().map(Path::to_path_buf)
                }
            })
            .or_else(|| {
                store
                    .get_emulator_exe_path(platform, Some(&profile.id))
                    .ok()
                    .flatten()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            })
            .unwrap_or_else(|| library_root.join("Emulators").join(platform));
        let _ = crate::commands::adopt_bundled_profile_system_files(
            &store,
            &state.data_dir,
            &library_root,
            &profile,
            &install_dir,
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn install_emulator(
    app: AppHandle,
    state: State<'_, AppState>,
    platform: String,
) -> Result<EmulatorInstallResult, String> {
    install_emulator_internal(&app, &state, platform.trim(), None, None).await
}

#[tauri::command]
pub fn get_emulator_status(
    state: State<'_, AppState>,
    platform: String,
) -> Result<EmulatorStatus, String> {
    get_emulator_status_internal(&state, platform.trim())
}

#[tauri::command]
pub fn get_emulator_install_status(
    state: State<'_, AppState>,
    platform: String,
) -> Result<EmulatorStatus, String> {
    get_emulator_status_internal(&state, platform.trim())
}

pub(crate) async fn install_emulator_internal(
    app: &AppHandle,
    state: &AppState,
    platform: &str,
    source_override: Option<EmulatorSourceOverride>,
    target_dir: Option<&Path>,
) -> Result<EmulatorInstallResult, String> {
    match load_emulator_profile(platform)? {
        Some(profile) => {
            install_profile_emulator(app, state, profile, source_override, target_dir).await
        }
        // Platforms without a bundled emulator profile (e.g. Switch) are
        // installed entirely from the manifest. Under the "Empty Shell" model
        // the user-supplied manifest is the source of truth for the emulator
        // URL and executable name; there is no built-in gate.
        None => install_manifest_emulator(app, state, platform, source_override, target_dir).await,
    }
}

/// Install an emulator that has a bundled platform profile (PS1, SNES, …). A
/// manifest `source_override` may replace the profile's resolved download URL
/// and hash; otherwise the profile's version strategy is used.
async fn install_profile_emulator(
    app: &AppHandle,
    state: &AppState,
    profile: EmulatorProfile,
    source_override: Option<EmulatorSourceOverride>,
    target_dir: Option<&Path>,
) -> Result<EmulatorInstallResult, String> {
    if let Some(asset_id) = source_override
        .as_ref()
        .and_then(|source| source.asset_id.as_deref())
    {
        let bundle_dir =
            commands::prepare_manifest_emulator_bundle_asset(asset_id, state, app, target_dir)
                .await?;
        if let Some((exe_path, version)) = existing_emulator(state, &profile, Some(&bundle_dir))? {
            if path_is_inside(&bundle_dir, Path::new(&exe_path)) {
                return Ok(EmulatorInstallResult {
                    profile_id: profile.id,
                    exe_path,
                    version,
                    from_cache: true,
                });
            }
        }

        let exe_path = resolve_bundle_executable(
            &bundle_dir,
            &profile.exe_relative_path,
            &profile.display_name,
        )?;
        persist_emulator(state, &profile, "manifest", &exe_path)?;
        return Ok(EmulatorInstallResult {
            profile_id: profile.id,
            exe_path: exe_path.to_string_lossy().to_string(),
            version: "manifest".to_string(),
            from_cache: false,
        });
    }

    let install_dir = emulator_profile_install_dir(state, &profile, target_dir);
    if let Some((exe_path, version)) =
        existing_emulator(state, &profile, target_dir.map(|_| install_dir.as_path()))?
    {
        return Ok(EmulatorInstallResult {
            profile_id: profile.id,
            exe_path,
            version,
            from_cache: true,
        });
    }

    let resolved = match &source_override {
        Some(source) => {
            let url = validate_repository_url(&source.url, false)
                .map_err(|error| format!("invalid_emulator_url:{error}"))?;
            ResolvedAsset {
                filename: file_name_from_url(url.as_str())
                    .unwrap_or_else(|| format!("{}.zip", profile.id)),
                url: url.to_string(),
                size: 0,
                version: "manifest".to_string(),
            }
        }
        None => resolve_profile_asset(state, &profile).await?,
    };
    if resolved.size > MAX_EMULATOR_ARCHIVE_BYTES {
        return Err(format!(
            "emulator_archive_too_large:{} is {} bytes",
            resolved.filename, resolved.size
        ));
    }

    emit_progress(
        app,
        "",
        "emulator",
        &format!("Downloading {}...", profile.display_name),
        10,
    );
    // A manifest-supplied hash takes precedence over the profile's pinned hash.
    let expected_sha256 = match source_override
        .as_ref()
        .and_then(|source| source.sha256.as_deref())
    {
        Some(sha) => Some(sha),
        None => match &profile.version_strategy {
            VersionStrategy::Fixed { sha256, .. } => Some(sha256.as_str()),
            VersionStrategy::GithubLatest { .. } => None,
        },
    };
    let archive_path =
        download_emulator_archive(state, &resolved, expected_sha256, &profile.display_name).await?;
    let exe_path = extract_emulator_archive(
        &archive_path,
        &install_dir,
        &profile.id,
        &profile.exe_relative_path,
        &profile.display_name,
    )
    .await?;
    persist_emulator(state, &profile, &resolved.version, &exe_path)?;
    if source_override
        .as_ref()
        .and_then(|source| source.asset_id.as_deref())
        .is_none()
    {
        let _ = fs::remove_file(archive_path);
    }

    Ok(EmulatorInstallResult {
        profile_id: profile.id,
        exe_path: exe_path.to_string_lossy().to_string(),
        version: resolved.version,
        from_cache: false,
    })
}

/// Install an emulator for a platform that has no bundled profile (Switch). The
/// download URL, executable name, and optional hash all come from the manifest.
/// The result is persisted under the platform's default setup profile
/// (`switch-manual` for Switch) so the launcher resolves it like any other
/// configured emulator.
async fn install_manifest_emulator(
    app: &AppHandle,
    state: &AppState,
    platform: &str,
    source_override: Option<EmulatorSourceOverride>,
    target_dir: Option<&Path>,
) -> Result<EmulatorInstallResult, String> {
    let source = source_override.ok_or_else(|| {
        format!(
            "emulator_source_missing:{platform} has no bundled emulator; the manifest must provide core_bundle_url."
        )
    })?;
    let executable = source
        .executable
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!("emulator_executable_unknown:{platform} manifest must set launch_config.executable.")
        })?
        .to_string();

    let default_profile = crate::setup_profiles::get_default_platform_setup_profile(platform);
    let profile_id = default_profile
        .as_ref()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| format!("{platform}-manifest"));
    let display_name = default_profile
        .as_ref()
        .map(|profile| profile.emulator.emulator_name.clone())
        .unwrap_or_else(|| format!("{platform} emulator"));

    if let Some(asset_id) = source.asset_id.as_deref() {
        let bundle_dir =
            commands::prepare_manifest_emulator_bundle_asset(asset_id, state, app, target_dir)
                .await?;
        let cached = {
            let store = lock_store(state)?;
            if let Some(path) = store.get_emulator_exe_path(platform, None)? {
                if path_is_inside(&bundle_dir, &path) {
                    let version = store
                        .get_emulator_config(platform)?
                        .and_then(|config| config.version)
                        .unwrap_or_else(|| "installed".to_string());
                    Some(EmulatorInstallResult {
                        profile_id: profile_id.clone(),
                        exe_path: path.to_string_lossy().to_string(),
                        version,
                        from_cache: true,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(result) = cached {
            return Ok(result);
        }

        let exe_path = resolve_bundle_executable(&bundle_dir, &executable, &display_name)?;
        let exe = exe_path.to_string_lossy().to_string();
        lock_store(state)?.upsert_emulator_config(
            platform,
            Some(&exe),
            "valid",
            Some("manifest"),
            None,
        )?;
        return Ok(EmulatorInstallResult {
            profile_id,
            exe_path: exe,
            version: "manifest".to_string(),
            from_cache: false,
        });
    }

    // Reuse only a managed manifest install. A previously selected external
    // emulator can be valid, but it cannot expose keys/BIOS bundled in this
    // manifest archive because the archive was never extracted.
    let install_dir = manifest_emulator_install_dir(state, platform, &profile_id, target_dir);
    let library_root = crate::commands::library_root_for_app_state(state);
    let cached = {
        let store = lock_store(state)?;
        if let Some(path) = store.get_emulator_exe_path(platform, None)? {
            if target_dir
                .map(|_| path_is_inside(&install_dir, &path))
                .unwrap_or_else(|| {
                    is_managed_manifest_emulator_path(
                        &[state.data_dir.as_path(), library_root.as_path()],
                        platform,
                        &path,
                    )
                })
            {
                let version = store
                    .get_emulator_config(platform)?
                    .and_then(|config| config.version)
                    .unwrap_or_else(|| "installed".to_string());
                Some(EmulatorInstallResult {
                    profile_id: profile_id.clone(),
                    exe_path: path.to_string_lossy().to_string(),
                    version,
                    from_cache: true,
                })
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(result) = cached {
        return Ok(result);
    }

    let url = validate_repository_url(&source.url, false)
        .map_err(|error| format!("invalid_emulator_url:{error}"))?;
    let resolved = ResolvedAsset {
        filename: file_name_from_url(url.as_str())
            .unwrap_or_else(|| format!("{platform}-emulator.zip")),
        url: url.to_string(),
        size: 0,
        version: "manifest".to_string(),
    };

    emit_progress(
        app,
        "",
        "emulator",
        &format!("Downloading {display_name}..."),
        10,
    );
    let archive_path =
        download_emulator_archive(state, &resolved, source.sha256.as_deref(), &display_name)
            .await?;
    let exe_path = extract_emulator_archive(
        &archive_path,
        &install_dir,
        &profile_id,
        &executable,
        &display_name,
    )
    .await?;

    // Persist under the platform's default profile (e.g. switch-manual) so
    // get_emulator_status_internal and launch_game both resolve it.
    lock_store(state)?.upsert_emulator_config(
        platform,
        Some(&exe_path.to_string_lossy()),
        "valid",
        Some(&resolved.version),
        None,
    )?;
    if source.asset_id.is_none() {
        let _ = fs::remove_file(archive_path);
    }

    Ok(EmulatorInstallResult {
        profile_id,
        exe_path: exe_path.to_string_lossy().to_string(),
        version: resolved.version,
        from_cache: false,
    })
}

/// True when `exe_path` lives under `<root>/Emulators/<platform>` for any of the
/// supplied content roots. Multiple roots are accepted so installs made before
/// the library root moved (old Roaming `data_dir`) keep counting as managed
/// alongside the current library root.
fn is_managed_manifest_emulator_path(roots: &[&Path], platform: &str, exe_path: &Path) -> bool {
    let Ok(exe_path) = fs::canonicalize(exe_path) else {
        return false;
    };
    roots.iter().any(|root| {
        let install_dir = root.join("Emulators").join(platform);
        fs::canonicalize(install_dir)
            .map(|install_dir| exe_path.starts_with(&install_dir))
            .unwrap_or(false)
    })
}

fn emulator_profile_install_dir(
    state: &AppState,
    profile: &EmulatorProfile,
    target_dir: Option<&Path>,
) -> PathBuf {
    target_dir
        .map(|dir| dir.join(crate::downloads::safe_segment(&profile.id)))
        .unwrap_or_else(|| {
            crate::commands::library_root_for_app_state(state)
                .join("Emulators")
                .join(&profile.platform)
                .join(&profile.id)
        })
}

fn manifest_emulator_install_dir(
    state: &AppState,
    platform: &str,
    profile_id: &str,
    target_dir: Option<&Path>,
) -> PathBuf {
    target_dir
        .map(|dir| dir.join(crate::downloads::safe_segment(profile_id)))
        .unwrap_or_else(|| {
            crate::commands::library_root_for_app_state(state)
                .join("Emulators")
                .join(platform)
                .join(profile_id)
        })
}

fn path_is_inside(root: &Path, path: &Path) -> bool {
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    path.starts_with(root)
}

fn resolve_bundle_executable(
    bundle_dir: &Path,
    exe_relative_path: &str,
    display_name: &str,
) -> Result<PathBuf, String> {
    let executable = archive::resolve_executable(bundle_dir, exe_relative_path, display_name)?;
    fs::canonicalize(&executable)
        .map_err(|error| format!("Failed to resolve emulator executable: {error}"))
}

/// Stage, extract, locate the executable, and atomically swap an emulator
/// archive into its managed install directory. Shared by the profile and
/// manifest install paths. Returns the absolute path to the resolved
/// executable.
async fn extract_emulator_archive(
    archive_path: &Path,
    install_dir: &Path,
    staging_label: &str,
    exe_relative_path: &str,
    display_name: &str,
) -> Result<PathBuf, String> {
    let install_root = install_dir.parent().map(Path::to_path_buf).ok_or_else(|| {
        format!(
            "Emulator install path has no parent: {}",
            install_dir.display()
        )
    })?;
    let staging_dir = install_root.join(format!(
        ".installing-{}",
        crate::downloads::safe_segment(staging_label)
    ));
    archive::reset_staging_dir(&install_root, &staging_dir)?;
    {
        // Archive extraction is synchronous and CPU/IO heavy; keep it off the
        // async runtime worker so concurrent commands stay responsive.
        let archive_path = archive_path.to_path_buf();
        let staging_dir = staging_dir.clone();
        tokio::task::spawn_blocking(move || {
            archive::extract_archive_safely(&archive_path, &staging_dir)
        })
        .await
        .map_err(|error| format!("Emulator extraction task failed: {error}"))??;
    }
    let staged_exe = archive::resolve_executable(&staging_dir, exe_relative_path, display_name)?;
    let relative_exe = staged_exe
        .strip_prefix(&staging_dir)
        .map_err(|_| "Installed emulator executable escaped the staging folder.".to_string())?
        .to_path_buf();
    archive::replace_directory(&install_root, install_dir, &staging_dir)?;
    let installed_exe = install_dir.join(relative_exe);
    fs::canonicalize(&installed_exe)
        .map_err(|error| format!("Failed to resolve installed emulator executable: {error}"))
}

fn get_emulator_status_internal(
    state: &AppState,
    platform: &str,
) -> Result<EmulatorStatus, String> {
    if platform == "switch" {
        let exe_path = lock_store(state)?.get_emulator_exe_path("switch", Some("switch-manual"))?;
        return Ok(EmulatorStatus {
            platform: platform.to_string(),
            installed: exe_path.is_some(),
            exe_path: exe_path.map(|path| path.to_string_lossy().to_string()),
            profile_id: Some("switch-manual".to_string()),
        });
    }

    let profile = load_emulator_profile(platform)?;
    let exe_path = match profile.as_ref() {
        Some(profile) => lock_store(state)?.get_emulator_exe_path(platform, Some(&profile.id))?,
        None => None,
    };
    Ok(EmulatorStatus {
        platform: platform.to_string(),
        installed: exe_path.is_some(),
        exe_path: exe_path.map(|path| path.to_string_lossy().to_string()),
        profile_id: profile.map(|profile| profile.id),
    })
}

async fn install_game_inner(
    app: &AppHandle,
    state: &AppState,
    game_id: &str,
    options: &ResolvedInstallGameOptions,
) -> Result<InstallResult, String> {
    let game = lock_store(state)?
        .get_game(game_id)?
        .ok_or_else(|| format!("unknown_game:{game_id}"))?;

    emit_progress(app, game_id, "emulator", "Checking emulator...", 5);
    // A manifest can ship the emulator (and its keys/BIOS) via core_bundle_url.
    // When present we install that bundle here, so the keys check below runs
    // only AFTER the archive is downloaded and extracted, and a missing local
    // copy of the keys never blocks the install.
    let emulator_bundle = manifest_emulator_override(state, game_id)?;
    // Group the emulator/core-bundle download under this game install so the
    // Downloads view shows them together instead of as two unrelated rows. The
    // link is declared upfront (before the download starts) so enrichment can
    // tag the row as soon as it appears.
    if let Some(asset_id) = emulator_bundle
        .as_ref()
        .and_then(|bundle| bundle.asset_id.as_deref())
    {
        lock_store(state)?.set_download_parent(asset_id, game_id)?;
    }
    if game.platform == "switch" {
        if let Some(bundle) = emulator_bundle.clone() {
            install_emulator_internal(
                app,
                state,
                &game.platform,
                Some(bundle),
                options.emulator_target_dir.as_deref(),
            )
            .await
            .map_err(|error| format!("emulator_install_failed:{error}"))?;
        } else if !get_emulator_status_internal(state, "switch")?.installed {
            return Ok(InstallResult {
                game_id: game_id.to_string(),
                status: "error".to_string(),
                error_code: Some("switch_emulator_not_configured".to_string()),
                message: Some(
                    "Select a Switch emulator executable before installing this game.".to_string(),
                ),
            });
        }
    } else {
        install_emulator_internal(
            app,
            state,
            &game.platform,
            emulator_bundle.clone(),
            options.emulator_target_dir.as_deref(),
        )
        .await
        .map_err(|error| format!("emulator_install_failed:{error}"))?;
    }
    // Register keys/BIOS that came inside the emulator archive before validating.
    adopt_bundled_system_files(
        state,
        &game.platform,
        emulator_bundle
            .as_ref()
            .and_then(|bundle| bundle.asset_id.as_deref()),
    )?;
    emit_progress(app, game_id, "emulator", "Emulator ready", 25);

    emit_progress(app, game_id, "system_files", "Checking system files...", 30);
    let content_dir = commands::library_root_for_app_state(state);
    let setup = {
        let store = lock_store(state)?;
        commands::build_game_setup_state(&store, &content_dir, &game)?
    };
    let missing = missing_system_files(&setup);
    if !missing.is_empty() {
        return Ok(InstallResult {
            game_id: game_id.to_string(),
            status: "needs_system_files".to_string(),
            error_code: Some(format!("missing:{}", missing.join(","))),
            message: Some(format!("Import once to continue: {}", missing.join(", "))),
        });
    }
    emit_progress(app, game_id, "system_files", "System files ready", 40);

    if setup.game_file.status != "ready" {
        if matches!(
            game.content_mode.as_deref(),
            Some("user_provided" | "metadata_only")
        ) || game
            .downloads
            .iter()
            .any(|source| matches!(source, SourceUri::UserProvided { .. }))
        {
            return Ok(InstallResult {
                game_id: game_id.to_string(),
                status: "needs_game_file".to_string(),
                error_code: Some("game_requires_import".to_string()),
                message: Some(
                    "Emulator installed. Import your local game file to continue.".to_string(),
                ),
            });
        }

        emit_progress(app, game_id, "game", "Downloading game...", 45);
        commands::start_game_download_internal(
            game_id,
            state,
            app,
            options.game_target_dir.as_deref(),
        )
        .await?;
        wait_for_game_download(app, state, game_id).await?;
    }
    emit_progress(app, game_id, "game", "Game downloaded", 90);

    emit_progress(app, game_id, "verify", "Verifying launch readiness...", 95);
    let final_content_dir = commands::library_root_for_app_state(state);
    let final_setup = {
        let store = lock_store(state)?;
        commands::build_game_setup_state(&store, &final_content_dir, &game)?
    };
    if final_setup.launch.status != "ready" {
        return Err(format!(
            "launch_not_ready:{}",
            final_setup.launch.blockers.join("; ")
        ));
    }
    emit_progress(app, game_id, "done", "Ready to play", 100);

    Ok(InstallResult {
        game_id: game_id.to_string(),
        status: "ready".to_string(),
        error_code: None,
        message: None,
    })
}

async fn wait_for_game_download(
    app: &AppHandle,
    state: &AppState,
    game_id: &str,
) -> Result<(), String> {
    loop {
        let Some(download) = state.torrents()?.get_game_download(game_id)? else {
            return Err(
                "download_state_missing: Download did not create a persisted record.".into(),
            );
        };
        match download.status.as_str() {
            "completed" => return Ok(()),
            "error" | "cancelled" => {
                return Err(format!(
                    "download_failed:{}",
                    download
                        .error_message
                        .unwrap_or_else(|| download.status.clone())
                ));
            }
            "paused" | "interrupted" => {
                return Err(format!("download_paused:{}", download.status));
            }
            _ => {
                let percent = 45 + ((download.progress_percent.clamp(0.0, 100.0) * 0.45) as u8);
                emit_progress(app, game_id, "game", "Downloading game...", percent);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

fn missing_system_files(setup: &crate::schema::GameSetupState) -> Vec<String> {
    let mut missing = setup
        .system_files
        .iter()
        .filter(|item| item.required && item.status != "ready")
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    missing.extend(
        setup
            .repository_requirements
            .iter()
            .filter(|item| item.status != "ready" || !item.trusted)
            .map(|item| item.asset.id.clone()),
    );
    missing
}

fn existing_emulator(
    state: &AppState,
    profile: &EmulatorProfile,
    required_root: Option<&Path>,
) -> Result<Option<(String, String)>, String> {
    let store = lock_store(state)?;
    if !store.is_emulator_installed(&profile.platform, Some(&profile.id))? {
        return Ok(None);
    }
    let path = store.get_emulator_exe_path(&profile.platform, Some(&profile.id))?;
    let version = store
        .get_profile_emulator_config(&profile.id)?
        .and_then(|config| config.version)
        .unwrap_or_else(|| "installed".to_string());
    if let Some(required_root) = required_root {
        if let Some(path) = path.as_deref() {
            if !path_is_inside(required_root, path) {
                return Ok(None);
            }
        }
    }
    Ok(path.map(|path| (path.to_string_lossy().to_string(), version)))
}

async fn resolve_profile_asset(
    state: &AppState,
    profile: &EmulatorProfile,
) -> Result<ResolvedAsset, String> {
    match &profile.version_strategy {
        VersionStrategy::GithubLatest {
            repo,
            asset_pattern,
        } => {
            let cache_key = format!("emulator_release:{}", profile.id);
            if let Some(cached) = lock_store(state)?
                .get_config(&cache_key)?
                .and_then(|value| serde_json::from_str::<CachedResolvedAsset>(&value).ok())
                .filter(|cached| {
                    Utc::now()
                        .signed_duration_since(cached.resolved_at)
                        .num_hours()
                        < RESOLVED_ASSET_CACHE_HOURS
                })
            {
                return Ok(cached.asset);
            }

            let asset = resolve_github_latest(repo, asset_pattern).await?;
            let cached = CachedResolvedAsset {
                resolved_at: Utc::now(),
                asset: asset.clone(),
            };
            lock_store(state)?.set_config(
                &cache_key,
                &serde_json::to_string(&cached).map_err(|error| error.to_string())?,
            )?;
            Ok(asset)
        }
        VersionStrategy::Fixed { url, .. } => Ok(ResolvedAsset {
            url: url.clone(),
            filename: file_name_from_url(url).unwrap_or_else(|| format!("{}.zip", profile.id)),
            size: 0,
            version: "fixed".to_string(),
        }),
    }
}

async fn download_emulator_archive(
    state: &AppState,
    asset: &ResolvedAsset,
    expected_sha256: Option<&str>,
    display_name: &str,
) -> Result<PathBuf, String> {
    let temp_dir =
        commands::temp_root(&commands::library_root_for_app_state(state)).join("emulators");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|error| format!("Failed to create emulator temp folder: {error}"))?;
    let archive_path = temp_dir.join(crate::downloads::safe_segment(&asset.filename));

    // When a hash is known (pinned profile or manifest-supplied) it is verified;
    // otherwise we still verify the reported size and cap the transfer so a
    // hostile redirect can't stream an unbounded archive.
    let expected_size_bytes = (asset.size > 0).then_some(asset.size);

    crate::downloads::download_http_streaming(
        &asset.url,
        &archive_path,
        crate::downloads::StreamOptions {
            expected_sha256,
            expected_size_bytes,
            max_bytes: Some(MAX_EMULATOR_ARCHIVE_BYTES),
            resume: true,
        },
        |_, _| {},
    )
    .await
    .map_err(|error| format!("Failed to download {display_name}: {error}"))?;

    Ok(archive_path)
}

fn persist_emulator(
    state: &AppState,
    profile: &EmulatorProfile,
    version: &str,
    exe_path: &Path,
) -> Result<(), String> {
    let exe = exe_path.to_string_lossy().to_string();
    let launch_args = profile.launch_args_template();
    let store = lock_store(state)?;
    store.upsert_profile_emulator_config(
        &profile.id,
        &profile.platform,
        Some(&exe),
        "valid",
        Some(version),
        Some(&launch_args),
    )?;
    Ok(())
}

fn resolve_install_game_options(
    options: Option<InstallGameOptions>,
) -> Result<ResolvedInstallGameOptions, String> {
    let Some(options) = options else {
        return Ok(ResolvedInstallGameOptions::default());
    };
    Ok(ResolvedInstallGameOptions {
        game_target_dir: commands::resolve_target_dir_override(options.game_target_dir.as_deref())?,
        emulator_target_dir: commands::resolve_target_dir_override(
            options.emulator_target_dir.as_deref(),
        )?,
    })
}

fn non_empty_executable(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn file_name_from_url(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .path_segments()?
        .rfind(|segment| !segment.is_empty())
        .map(ToString::to_string)
}

fn emit_progress(app: &AppHandle, game_id: &str, stage: &str, message: &str, percent: u8) {
    let _ = app.emit(
        "install:progress",
        InstallProgressEvent {
            game_id: game_id.to_string(),
            stage: stage.to_string(),
            message: message.to_string(),
            percent: percent.min(100),
        },
    );
}

fn error_code(error: &str) -> String {
    error
        .split_once(':')
        .map(|(code, _)| code)
        .unwrap_or("install_failed")
        .to_string()
}

fn lock_store(state: &AppState) -> Result<std::sync::MutexGuard<'_, RepositoryStore>, String> {
    state
        .store
        .lock()
        .map_err(|_| "Repository store lock is poisoned.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_emulator_config_key_is_namespaced_per_game() {
        assert_eq!(
            manifest_emulator_config_key("repo::game"),
            "manifest_emulator:repo::game"
        );
    }

    #[test]
    fn persisted_emulator_bundle_round_trips_through_json() {
        // This is exactly what persist/override store and read back via the
        // app_config table. Optional fields survive a round-trip and absent
        // ones default to None.
        let json = serde_json::to_string(&PersistedEmulatorBundle {
            url: "https://example.com/eden.zip".to_string(),
            sha256: Some("deadbeef".to_string()),
            asset_id: Some("repo::asset".to_string()),
            executable: Some("eden.exe".to_string()),
        })
        .unwrap();
        let back: PersistedEmulatorBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back.url, "https://example.com/eden.zip");
        assert_eq!(back.sha256.as_deref(), Some("deadbeef"));
        assert_eq!(back.asset_id.as_deref(), Some("repo::asset"));
        assert_eq!(back.executable.as_deref(), Some("eden.exe"));

        let minimal: PersistedEmulatorBundle =
            serde_json::from_str(r#"{"url":"https://example.com/x.zip"}"#).unwrap();
        assert!(minimal.sha256.is_none());
        assert!(minimal.asset_id.is_none());
        assert!(minimal.executable.is_none());
    }

    #[test]
    fn manifest_emulator_cache_reuses_only_managed_install_paths() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("Emulators").join("switch").join("eden.exe");
        let external = dir.path().join("External").join("eden.exe");
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::create_dir_all(external.parent().unwrap()).unwrap();
        std::fs::write(&managed, b"exe").unwrap();
        std::fs::write(&external, b"exe").unwrap();

        assert!(is_managed_manifest_emulator_path(
            &[dir.path()],
            "switch",
            &managed
        ));
        assert!(!is_managed_manifest_emulator_path(
            &[dir.path()],
            "switch",
            &external
        ));
        assert!(!is_managed_manifest_emulator_path(
            &[dir.path()],
            "switch",
            &dir.path()
                .join("Emulators")
                .join("switch")
                .join("missing.exe")
        ));

        // A second (legacy) root is also honoured.
        let legacy = tempfile::tempdir().unwrap();
        let legacy_exe = legacy
            .path()
            .join("Emulators")
            .join("switch")
            .join("eden.exe");
        std::fs::create_dir_all(legacy_exe.parent().unwrap()).unwrap();
        std::fs::write(&legacy_exe, b"exe").unwrap();
        assert!(is_managed_manifest_emulator_path(
            &[dir.path(), legacy.path()],
            "switch",
            &legacy_exe
        ));
    }

    #[test]
    fn manifest_bundle_executable_resolves_to_absolute_emulator_path() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_dir = dir.path().join("Emulators").join("switch").join("bundle");
        let nested = bundle_dir.join("Eden");
        std::fs::create_dir_all(&nested).unwrap();
        let exe = nested.join("eden.exe");
        std::fs::write(&exe, b"exe").unwrap();

        let resolved = resolve_bundle_executable(&bundle_dir, "eden.exe", "Eden").unwrap();

        assert!(resolved.is_absolute());
        assert_eq!(resolved, std::fs::canonicalize(exe).unwrap());
    }

    #[test]
    fn manifest_asset_cache_scope_rejects_stale_emulators_install() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_dir = dir.path().join("Emulators").join("switch").join("bundle");
        let bundle_exe = bundle_dir.join("eden.exe");
        let stale_exe = dir.path().join("Emulators").join("switch").join("eden.exe");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        std::fs::create_dir_all(stale_exe.parent().unwrap()).unwrap();
        std::fs::write(&bundle_exe, b"exe").unwrap();
        std::fs::write(&stale_exe, b"old").unwrap();

        assert!(path_is_inside(&bundle_dir, &bundle_exe));
        assert!(!path_is_inside(&bundle_dir, &stale_exe));
    }
}
