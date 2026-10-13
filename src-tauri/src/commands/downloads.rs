#[tauri::command]
pub async fn download_asset(
    asset_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DownloadRecord, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
    download_asset_internal(&asset_id, state.inner(), &app, target_dir.as_deref()).await
}

pub(crate) async fn download_asset_internal(
    asset_id: &str,
    state: &AppState,
    app: &AppHandle,
    target_dir: Option<&Path>,
) -> Result<DownloadRecord, String> {
    let content_dir = library_root_for_app_state(state);
    let asset = {
        let store = lock_app_store(state)?;
        store
            .get_asset(asset_id)?
            .ok_or_else(|| format!("Unknown asset: {asset_id}"))?
    };
    // Reserve before target creation and before any failure/status write.
    let lease = state.direct_downloads.start(&asset.id)?;
    let identity = crate::storage::DownloadAttempt {
        subject: asset.id.clone(),
        attempt: lease.attempt().to_string(),
        run: 1,
    };
    let source = asset
        .sources
        .first()
        .ok_or_else(|| format!("Asset {} has no sources.", asset.display_name))?;
    let destination = {
        let store = lock_app_store(state)?;
        resolve_asset_target_with_override(&store, &content_dir, &asset, source, target_dir)
            .map_err(|blocked| blocked.message)?
    };
    if matches!(source, SourceUri::UserProvided { .. }) {
        return Err(format!(
            "{} is user-provided. Place the file at {} or use Import file.",
            asset.display_name,
            destination.display()
        ));
    }
    let is_bundle = is_manifest_emulator_bundle(&asset);
    // System files retain their required emulator-relative path. Never replace
    // a preexisting user file, including a symlink or an earlier installation.
    if !is_bundle && fs::symlink_metadata(&destination).is_ok() {
        return Err(format!(
            "Asset destination already exists; existing file preserved: {}",
            destination.display()
        ));
    }
    let parent = destination
        .parent()
        .ok_or("Asset target has no parent directory.")?;
    let staging_parent = if is_bundle {
        parent.to_path_buf()
    } else {
        parent.join(".fusion-staging")
    };
    let ownership = {
        let store = lock_app_store(state)?;
        crate::download_installation::allocate(&store, &asset.id, &lease, &staging_parent)?
    };
    let staged_destination = ownership.root().join(
        destination
            .file_name()
            .ok_or("Asset target has no filename.")?,
    );
    let target_path = if is_bundle {
        staged_destination.to_string_lossy().to_string()
    } else {
        destination.to_string_lossy().to_string()
    };
    let expected_bytes = source_size_bytes(source).unwrap_or(0);
    let started = {
        let store = lock_app_store(state)?;
        store.record_direct_asset_download_started(
            &asset.id,
            direct_source_kind(source),
            &target_path,
            expected_bytes,
        )?;
        store.assign_download_attempt(&identity)?;
        store
            .get_torrent_download(&asset.id)?
            .ok_or("Asset download state disappeared.")?
    };
    emit_direct_download_record(app, &started);
    let cancel_token = lease.token();
    let result: Result<DownloadRecord, String> = async {
        let asset_id = asset.id.clone();
        let asset_name = asset.display_name.clone();
        let progress_identity = identity.clone();
        let progress_lease = lease.clone();
        let progress_target = target_path.clone();
        let mut speed_meter = crate::downloads::DirectDownloadSpeedMeter::new();
        let mut last_sqlite_flush = std::time::Instant::now();
        let file = crate::downloads::download_source_to_file_guarded(
            source,
            &staged_destination,
            move |downloaded, total| {
                if progress_lease.is_cancelled() {
                    return;
                }
                let speed = speed_meter.sample(downloaded);
                let percent = total
                    .filter(|total| *total > 0)
                    .map_or(0.0, |total| downloaded as f64 / total as f64 * 100.0);
                if last_sqlite_flush.elapsed() >= std::time::Duration::from_millis(1500) {
                    last_sqlite_flush = std::time::Instant::now();
                    if let Ok(store) = lock_app_store(state) {
                        if let Ok(record) = store.attempt_progress(
                            &progress_identity,
                            crate::storage::WorkerProgress {
                                percent,
                                downloaded,
                                total: total.unwrap_or(0),
                                speed,
                                upload: 0,
                                peers: 0,
                            },
                        ) {
                            drop(store);
                            emit_direct_download_record(app, &record);
                        }
                    }
                    return;
                }
                emit_direct_download_record(
                    app,
                    &TorrentDownloadRecord {
                        attempt_id: Some(progress_identity.attempt.clone()),
                        run_id: Some(progress_identity.run),
                        game_id: asset_id.clone(),
                        subject_type: Some("asset".to_string()),
                        display_name: Some(asset_name.clone()),
                        parent_game_id: None,
                        magnet_uri: String::new(),
                        save_dir: progress_target.clone(),
                        status: "downloading".to_string(),
                        progress_percent: percent,
                        downloaded_bytes: downloaded,
                        total_bytes: total.unwrap_or(0),
                        download_speed_bytes_per_sec: speed,
                        upload_speed_bytes_per_sec: 0,
                        peers_count: 0,
                        torrent_id: None,
                        error_message: None,
                        created_at: String::new(),
                        updated_at: String::new(),
                        completed_at: None,
                    },
                );
            },
            &lease,
        )
        .await?;
        let total_bytes = source_size_bytes(source).unwrap_or_else(|| file_size(&file.path));
        let final_path = if is_bundle {
            post_process_manifest_emulator_bundle_archive(
                &asset,
                &file.path,
                &file.sha256,
                lease.clone(),
            )
            .await?
        } else {
            install_staged_asset_without_overwrite(
                &file.path,
                &destination,
                &file.sha256,
                lease.clone(),
            )
            .await?
        };
        if lease.is_cancelled() {
            return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
        }
        crate::download_installation::snapshot(
            state.store.clone(),
            asset.id.clone(),
            lease.clone(),
        )
        .await?;
        record_completed_asset_download(
            state,
            app,
            &identity,
            &lease,
            &final_path,
            Some(&file.sha256),
            total_bytes,
        )
    }
    .await;
    if result.is_err() {
        // Capture partial staging artifacts after our writers have returned.
        let _ = crate::download_installation::snapshot(
            state.store.clone(),
            asset.id.clone(),
            lease.clone(),
        )
        .await;
    }
    match result {
        Ok(record) => Ok(record),
        Err(error)
            if cancel_token.is_cancelled()
                || crate::downloads::is_download_cancelled_error(&error) =>
        {
            // Removal owns the row. Keep its generation/state until every writer exits.
            Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.to_string())
        }
        Err(error) => {
            if let Ok(store) = lock_app_store(state) {
                if let Ok(record) = store.fail_attempt(&identity, "asset", &error) {
                    drop(store);
                    emit_direct_download_record(app, &record);
                }
            }
            Err(error)
        }
    }
}

fn record_completed_asset_download(
    state: &AppState,
    app: &AppHandle,
    identity: &crate::storage::DownloadAttempt,
    lease: &crate::download_operations::OperationLease,
    local_path: &Path,
    sha256: Option<&str>,
    total_bytes: u64,
) -> Result<DownloadRecord, String> {
    if lease.is_cancelled() {
        return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
    }
    state.direct_downloads.ensure_active(lease)?;
    let store = lock_app_store(state)?;
    let (record, torrent) = store.complete_attempt(
        identity,
        "asset",
        &local_path.to_string_lossy(),
        sha256,
        total_bytes,
    )?;
    drop(store);
    emit_direct_download_record(app, &torrent);
    Ok(record)
}

async fn install_staged_asset_without_overwrite(
    staged_path: &Path,
    final_path: &Path,
    expected_sha256: &str,
    lease: crate::download_operations::OperationLease,
) -> Result<PathBuf, String> {
    let staged_path = staged_path.to_path_buf();
    let final_path = final_path.to_path_buf();
    let expected_sha256 = expected_sha256.to_string();
    tokio::task::spawn_blocking(move || {
        let _protection = lease.protect_writes()?;
        publish_staged_asset_without_overwrite(&staged_path, &final_path, &expected_sha256, || {
            lease.is_cancelled()
        })
    })
    .await
    .map_err(|error| format!("Asset installation task failed: {error}"))?
}

fn publish_staged_asset_without_overwrite(
    staged_path: &Path,
    final_path: &Path,
    expected_sha256: &str,
    is_cancelled: impl Fn() -> bool,
) -> Result<PathBuf, String> {
    if is_cancelled() {
        return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
    }
    let parent = staged_path.parent().ok_or("Staged asset has no parent.")?;
    let _source_guard = crate::install_ownership::lock_plain_directory(parent)?;
    let file = open_verified_download(staged_path, expected_sha256, &is_cancelled)?;
    if is_cancelled() {
        return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
    }
    crate::install_ownership::publish_rename_no_replace(&file, final_path)?;
    // Required system-file destinations are outside owned staging and remain
    // preserved/reported by removal until they have separate ownership records.
    Ok(final_path.to_path_buf())
}

fn open_verified_download(
    path: &Path,
    expected: &str,
    is_cancelled: &impl Fn() -> bool,
) -> Result<fs::File, String> {
    use std::io::Read;
    let mut file = crate::install_ownership::open_publication_file(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if is_cancelled() {
            return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
        }
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if !hex::encode(hash.finalize()).eq_ignore_ascii_case(expected) {
        return Err("Downloaded file changed before installation.".into());
    }
    Ok(file)
}

async fn post_process_manifest_emulator_bundle_archive(
    asset: &AssetView,
    archive_path: &Path,
    expected_sha256: &str,
    lease: crate::download_operations::OperationLease,
) -> Result<PathBuf, String> {
    let asset = asset.clone();
    let archive_path = archive_path.to_path_buf();
    let expected_sha256 = expected_sha256.to_string();
    tokio::task::spawn_blocking(move || {
        if !is_manifest_emulator_bundle(&asset) {
            return Err("Asset is not a manifest emulator bundle.".into());
        }
        if !archive_path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            return Err(format!(
                "unsupported_manifest_emulator_archive:{}",
                archive_path.display()
            ));
        }
        let extract_dir = archive_path
            .parent()
            .ok_or("Emulator archive has no parent.")?
            .to_path_buf();
        let _protection = lease.protect_writes()?;
        let verified =
            open_verified_download(&archive_path, &expected_sha256, &|| lease.is_cancelled())?;
        crate::archive::extract_archive_safely_guarded(&archive_path, &extract_dir, lease.clone())?;
        if lease.is_cancelled() {
            return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
        }
        crate::install_ownership::delete_open_file(&verified)?;
        Ok(extract_dir)
    })
    .await
    .map_err(|error| format!("Emulator bundle extraction task failed: {error}"))?
}
#[cfg(test)]
pub(super) fn extract_manifest_emulator_bundle_archive(
    asset: &AssetView,
    archive_path: &Path,
) -> Result<PathBuf, String> {
    if !is_manifest_emulator_bundle(asset) {
        return Err(format!(
            "asset_is_not_manifest_emulator_bundle:{}",
            asset.id
        ));
    }
    let extension = archive_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("zip") {
        return Err(format!(
            "unsupported_manifest_emulator_archive:{}",
            archive_path.display()
        ));
    }
    let extract_dir = archive_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("Emulator archive has no parent: {}", archive_path.display()))?;
    crate::archive::extract_archive_safely(archive_path, &extract_dir)?;
    fs::remove_file(archive_path)
        .map_err(|error| format!("Failed to remove emulator zip archive: {error}"))?;
    Ok(extract_dir)
}

#[cfg(test)]
mod asset_download_lifecycle_tests {
    use super::*;

    #[test]
    fn staged_asset_replacement_after_download_is_not_published() {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged.bin");
        let target = temp.path().join("system.bin");
        fs::write(&staged, b"verified").unwrap();
        let hash = hash_file(&staged).unwrap();
        fs::write(&staged, b"substituted").unwrap();
        assert!(publish_staged_asset_without_overwrite(&staged, &target, &hash, || false).is_err());
        assert!(!target.exists());
    }

    #[test]
    fn asset_publication_cancelled_at_final_boundary_creates_no_target() {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged.bin");
        let target = temp.path().join("system.bin");
        fs::write(&staged, b"complete download").unwrap();
        let checks = std::cell::Cell::new(0);
        let result = publish_staged_asset_without_overwrite(
            &staged,
            &target,
            &hash_file(&staged).unwrap(),
            || {
                checks.set(checks.get() + 1);
                checks.get() == 2
            },
        );
        assert_eq!(
            result.unwrap_err(),
            crate::downloads::DOWNLOAD_CANCELLED_MESSAGE
        );
        assert!(!target.exists());
        assert_eq!(fs::read(&staged).unwrap(), b"complete download");
    }

    #[tokio::test]
    async fn asset_publication_failure_creates_no_partial_target() {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged.bin");
        let target = temp.path().join("missing-parent").join("system.bin");
        fs::write(&staged, b"complete download").unwrap();
        let registry = crate::download_operations::OperationCoordinator::new();
        let result = install_staged_asset_without_overwrite(
            &staged,
            &target,
            &hash_file(&staged).unwrap(),
            registry.start("asset").unwrap(),
        )
        .await;
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read(&staged).unwrap(), b"complete download");
    }

    #[tokio::test]
    async fn staged_asset_install_preserves_existing_user_target() {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged.bin");
        let target = temp.path().join("system.bin");
        fs::write(&staged, b"download").unwrap();
        fs::write(&target, b"user sentinel").unwrap();
        let registry = crate::download_operations::OperationCoordinator::new();
        let lease = registry.start("asset").unwrap();
        let result = install_staged_asset_without_overwrite(
            &staged,
            &target,
            &hash_file(&staged).unwrap(),
            lease,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"user sentinel");
    }

    #[tokio::test]
    async fn cancelled_staged_asset_install_does_not_create_target() {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged.bin");
        let target = temp.path().join("system.bin");
        fs::write(&staged, b"download").unwrap();
        let registry = crate::download_operations::OperationCoordinator::new();
        let lease = registry.start("asset").unwrap();
        registry.cancel("asset").unwrap();
        let error = install_staged_asset_without_overwrite(
            &staged,
            &target,
            &hash_file(&staged).unwrap(),
            lease,
        )
        .await
        .unwrap_err();
        assert_eq!(error, crate::downloads::DOWNLOAD_CANCELLED_MESSAGE);
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn staged_asset_install_moves_verified_file_to_required_target() {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged.bin");
        let target = temp.path().join("system.bin");
        let bytes = vec![42_u8; 128 * 1024];
        fs::write(&staged, &bytes).unwrap();
        let registry = crate::download_operations::OperationCoordinator::new();
        let lease = registry.start("asset").unwrap();
        assert_eq!(
            install_staged_asset_without_overwrite(
                &staged,
                &target,
                &hash_file(&staged).unwrap(),
                lease
            )
            .await
            .unwrap(),
            target
        );
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert!(!staged.exists());
    }
}

#[cfg(test)]
pub(super) fn extract_game_archive_if_needed(
    game: &CatalogGameView,
    downloaded_path: &Path,
) -> Result<PathBuf, String> {
    let extension = downloaded_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();

    if !extension.eq_ignore_ascii_case("zip") && !extension.eq_ignore_ascii_case("7z") {
        return Ok(downloaded_path.to_path_buf());
    }

    if game.expected_extensions.iter().any(|ext| {
        ext.eq_ignore_ascii_case(".zip")
            || ext.eq_ignore_ascii_case("zip")
            || ext.eq_ignore_ascii_case(".7z")
            || ext.eq_ignore_ascii_case("7z")
    }) {
        return Ok(downloaded_path.to_path_buf());
    }

    let extract_dir = downloaded_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("Game archive has no parent: {}", downloaded_path.display()))?;

    crate::archive::extract_archive_safely(downloaded_path, &extract_dir)?;
    let _ = fs::remove_file(downloaded_path);

    if let Ok(normalized_extensions) =
        crate::game_files::normalize_expected_extensions(&game.expected_extensions)
    {
        if let Ok(resolved) = crate::game_files::resolve_game_path(
            &extract_dir,
            &normalized_extensions,
            game.launch
                .as_ref()
                .and_then(|launch| launch.preferred_file.as_deref()),
        ) {
            return Ok(resolved);
        }
    }

    Ok(extract_dir)
}

#[tauri::command]
pub fn import_asset_file(
    asset_id: String,
    source_path: String,
    state: State<'_, AppState>,
) -> Result<ImportAssetFileReport, String> {
    let _lease = state.direct_downloads.start(&asset_id)?;
    let content_dir = library_root_for_app_state(&state);
    let store = match lock_store(&state) {
        Ok(store) => store,
        Err(_) => {
            return Ok(import_asset_error("", "store_failed"));
        }
    };

    Ok(import_asset_file_into_store(
        &store,
        &content_dir,
        &asset_id,
        Path::new(source_path.trim()),
    ))
}

#[tauri::command]
pub fn import_game_file(
    game_id: String,
    source_path: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ImportGameFileReport, String> {
    let lease = state.direct_downloads.start(&game_id)?;
    let download_root = match download_root(&state) {
        Ok(path) => path,
        Err(_) => return Ok(import_game_error(&game_id, "", "store_failed", None)),
    };
    let store = match lock_store(&state) {
        Ok(store) => store,
        Err(_) => return Ok(import_game_error("", "", "store_failed", None)),
    };

    let report = import_game_file_into_store(
        &store,
        &download_root,
        &game_id,
        Path::new(source_path.trim()),
    );
    if report.error_code.is_none() {
        store.assign_download_attempt(&crate::storage::DownloadAttempt {
            subject: game_id.clone(),
            attempt: lease.attempt().into(),
            run: 1,
        })?;
        spawn_metadata_scrape(&app, state.inner(), report.game_id.clone());
    }
    Ok(report)
}

include!("game_download_runtime.rs");

#[cfg(test)]
fn persist_direct_download_progress(
    store: &RepositoryStore,
    subject_id: &str,
    downloaded: u64,
    total: Option<u64>,
    download_speed_bytes_per_sec: u64,
) -> Result<TorrentDownloadRecord, String> {
    let progress_percent = match total {
        Some(total) if total > 0 => (downloaded as f64 / total as f64) * 100.0,
        _ => 0.0,
    };

    store.update_torrent_progress(
        subject_id,
        "downloading",
        progress_percent,
        downloaded,
        total.unwrap_or(0),
        download_speed_bytes_per_sec,
        0,
        0,
    )
}

fn game_torrent_save_dir(
    default_root: &Path,
    target_dir: Option<&Path>,
    game: &CatalogGameView,
) -> PathBuf {
    match target_dir {
        Some(target_dir) => target_dir.join(crate::downloads::safe_segment(&game.id)),
        None => default_root
            .join(crate::downloads::safe_segment(&game.platform))
            .join(crate::downloads::safe_segment(&game.id)),
    }
}

#[tauri::command]
pub fn trust_executable(
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<TrustedExecutable, String> {
    let store = lock_store(&state)?;
    let asset = store
        .get_asset(&asset_id)?
        .ok_or_else(|| format!("Unknown asset: {asset_id}"))?;
    if !asset.executable {
        return Err(format!(
            "Asset {} is not marked executable.",
            asset.display_name
        ));
    }

    let download = store.get_download(&asset.id)?.ok_or_else(|| {
        format!(
            "Executable asset {} has not been downloaded.",
            asset.display_name
        )
    })?;
    if !is_download_ready_status(&download.status) {
        return Err(format!(
            "Executable asset {} is not ready.",
            asset.display_name
        ));
    }

    let local_path = download
        .local_path
        .ok_or_else(|| format!("Executable asset {} has no local path.", asset.display_name))?;
    let sha256 = download.sha256.ok_or_else(|| {
        format!(
            "Executable asset {} has no verified SHA-256.",
            asset.display_name
        )
    })?;
    if !Path::new(&local_path).exists() {
        return Err(format!("Executable file is missing: {local_path}"));
    }

    store.trust_executable(&asset.id, &local_path, &sha256)
}

include!("download_removal.rs");

#[tauri::command]
pub async fn redownload_asset(
    asset_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DownloadRecord, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
    download_asset_internal(&asset_id, state.inner(), &app, target_dir.as_deref()).await
}

#[tauri::command]
pub fn open_game_folder(game_id: String, state: State<'_, AppState>) -> Result<(), String> {
    open_download_folder_internal(&game_id, state.inner())
}

#[tauri::command]
pub fn open_download_folder(download_id: String, state: State<'_, AppState>) -> Result<(), String> {
    open_download_folder_internal(&download_id, state.inner())
}

fn open_download_folder_internal(download_id: &str, state: &AppState) -> Result<(), String> {
    let path = {
        let store = lock_app_store(state)?;
        store
            .get_download(download_id)?
            .and_then(|download| download.local_path)
            .or_else(|| {
                store
                    .get_torrent_download(download_id)
                    .ok()
                    .flatten()
                    .map(|record| record.save_dir)
            })
            .ok_or_else(|| format!("Download is not available: {download_id}"))?
    };
    open_path(&folder_path_for_open(Path::new(&path))?)
}
