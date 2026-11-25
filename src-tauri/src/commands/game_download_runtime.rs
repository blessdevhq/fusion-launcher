#[tauri::command]
pub async fn download_game(
    game_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DownloadRecord, String> {
    // Legacy synchronous API shares the direct worker; it must never bypass leases.
    let game = lock_app_store(&state)?
        .get_game(&game_id)?
        .ok_or("Unknown game.")?;
    if game
        .downloads
        .first()
        .is_some_and(|s| matches!(s, SourceUri::Magnet { .. }))
    {
        return Err("Use start_game_download for magnet sources.".into());
    }
    let target = resolve_target_dir_override(target_dir.as_deref())?;
    start_game_download_internal(&game_id, &state, &app, target.as_deref())
        .await?
        .record
        .ok_or("Download did not produce a completed record.".into())
}

#[tauri::command]
pub async fn start_game_download(
    game_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<GameDownloadStartReport, String> {
    let target = resolve_target_dir_override(target_dir.as_deref())?;
    start_game_download_internal(&game_id, &state, &app, target.as_deref()).await
}

pub(crate) async fn start_game_download_internal(
    game_id: &str,
    state: &AppState,
    app: &AppHandle,
    target_dir: Option<&Path>,
) -> Result<GameDownloadStartReport, String> {
    let game = lock_app_store(state)?
        .get_game(game_id)?
        .ok_or_else(|| format!("Unknown game: {game_id}"))?;
    let source = game
        .downloads
        .first()
        .ok_or_else(|| format!("Game {} has no download sources.", game.title))?;
    if matches!(
        game.content_mode.as_deref(),
        Some("user_provided" | "metadata_only")
    ) || matches!(source, SourceUri::UserProvided { .. })
    {
        return Err(format!(
            "{} is user-provided content. Import your local game file.",
            game.title
        ));
    }
    let root = download_root_for_app_state(state)?;
    if let SourceUri::Magnet { uri, .. } = source {
        let parent = game_torrent_save_dir(&root, target_dir, &game);
        let torrent = state
            .torrents()?
            .start_magnet_download(
                game.id.clone(),
                uri.clone(),
                parent.to_string_lossy().to_string(),
            )
            .await?;
        return Ok(GameDownloadStartReport {
            game_id: game.id,
            source_kind: "magnet".into(),
            save_dir: torrent.save_dir,
            record: None,
            torrent: state.torrents()?.get_game_download(game_id)?,
        });
    }

    let lease = state.direct_downloads.start(&game.id)?;
    let identity = crate::storage::DownloadAttempt {
        subject: game.id.clone(),
        attempt: lease.attempt().into(),
        run: 1,
    };
    let parent = target_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(crate::downloads::safe_segment(&game.platform)));
    preflight_disk_space(&parent, source_size_bytes(source))?;
    let installation = {
        let store = lock_app_store(state)?;
        crate::download_installation::allocate(&store, &game.id, &lease, &parent)?
    };
    let destination = installation
        .root()
        .join(file_name_for_source(source, &game.title));
    let path = destination.to_string_lossy().to_string();
    let kind = direct_source_kind(source);
    let expected = source_size_bytes(source).unwrap_or(0);
    let started = {
        let store = lock_app_store(state)?;
        store.record_direct_game_download_started(&game.id, kind, &path, expected)?;
        store.assign_download_attempt(&identity)?;
        store
            .get_torrent_download(&game.id)?
            .ok_or("Download disappeared.")?
    };
    emit_direct_download_record(app, &started);
    let progress_identity = identity.clone();
    let token = lease.token();
    let mut speed = crate::downloads::DirectDownloadSpeedMeter::new();
    let result = crate::downloads::download_source_to_file_guarded(
        source,
        &destination,
        |downloaded, total| {
            if token.is_cancelled() {
                return;
            }
            let p = crate::storage::WorkerProgress {
                percent: total
                    .filter(|t| *t > 0)
                    .map(|t| downloaded as f64 / t as f64 * 100.0)
                    .unwrap_or(0.0),
                downloaded,
                total: total.unwrap_or(0),
                speed: speed.sample(downloaded),
                upload: 0,
                peers: 0,
            };
            if let Ok(store) = lock_app_store(state) {
                if let Ok(record) = store.attempt_progress(&progress_identity, p) {
                    drop(store);
                    emit_direct_download_record(app, &record);
                }
            }
        },
        &lease,
    )
    .await;
    let file = match result {
        Ok(file) => file,
        Err(error) => {
            let _ = crate::download_installation::snapshot(
                Arc::clone(&state.store),
                game.id.clone(),
                lease.clone(),
            )
            .await;
            finish_game_worker_error(state, app, &identity, &lease, &error)?;
            return Err(error);
        }
    };
    let bytes = source_size_bytes(source).unwrap_or_else(|| file_size(&file.path));
    let processed =
        post_process_game_archive_guarded(&game, &file.path, &file.sha256, lease.clone()).await;
    // Capture only this fresh installation, while its lease still excludes cleanup.
    let snapshot = crate::download_installation::snapshot(
        Arc::clone(&state.store),
        game.id.clone(),
        lease.clone(),
    )
    .await;
    let processed = match processed.and_then(|path| snapshot.map(|()| path)) {
        Ok(path) => path,
        Err(error) => {
            finish_game_worker_error(state, app, &identity, &lease, &error)?;
            return Err(error);
        }
    };
    if lease.is_cancelled() {
        let record = lock_app_store(state)?.attempt_status(
            &identity,
            &["downloading", "cancelling"],
            "cancelled",
            None,
        )?;
        emit_direct_download_record(app, &record);
        return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
    }
    let path = processed.to_string_lossy().to_string();
    let completion = lock_app_store(state)?.complete_attempt(
        &identity,
        "game",
        &path,
        Some(&file.sha256),
        bytes,
    );
    let (record, torrent) = match completion {
        Ok(completed) => completed,
        Err(error) => {
            finish_game_worker_error(state, app, &identity, &lease, &error)?;
            return Err(error);
        }
    };
    emit_direct_download_record(app, &torrent);
    spawn_metadata_scrape(app, state, game.id.clone());
    Ok(GameDownloadStartReport {
        game_id: game.id,
        source_kind: kind.into(),
        save_dir: path,
        record: Some(record),
        torrent: Some(torrent),
    })
}

fn finish_game_worker_error(
    state: &AppState,
    app: &AppHandle,
    identity: &crate::storage::DownloadAttempt,
    lease: &crate::download_operations::OperationLease,
    error: &str,
) -> Result<(), String> {
    let record = {
        let store = lock_app_store(state)?;
        if lease.is_cancelled() || crate::downloads::is_download_cancelled_error(error) {
            store.attempt_status(identity, &["downloading", "cancelling"], "cancelled", None)
        } else {
            store.fail_attempt(identity, "game", error)
        }
    };
    match record {
        Ok(record) => {
            emit_direct_download_record(app, &record);
            Ok(())
        }
        // A superseded callback must leave the successor's state untouched.
        Err(error) if error == "download_superseded" => Ok(()),
        Err(error) => Err(error),
    }
}

async fn post_process_game_archive_guarded(
    game: &CatalogGameView,
    path: &Path,
    expected_sha256: &str,
    lease: crate::download_operations::OperationLease,
) -> Result<PathBuf, String> {
    let game = game.clone();
    let path = path.to_path_buf();
    let expected_sha256 = expected_sha256.to_string();
    tokio::task::spawn_blocking(move || {
        if lease.is_cancelled() {
            return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !matches!(ext.to_ascii_lowercase().as_str(), "zip" | "7z")
            || game
                .expected_extensions
                .iter()
                .any(|s| s.trim_start_matches('.').eq_ignore_ascii_case(ext))
        {
            return Ok(path);
        }
        let root = path.parent().ok_or("Archive has no parent.")?;
        let _protection = lease.protect_writes()?;
        let verified = open_verified_download(&path, &expected_sha256, &|| lease.is_cancelled())?;
        crate::archive::extract_archive_safely_guarded(&path, root, lease.clone())?;
        if lease.is_cancelled() {
            return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
        }
        let extensions = game_files::normalize_expected_extensions(&game.expected_extensions)?;
        let resolved = game_files::resolve_game_path(
            root,
            &extensions,
            game.launch
                .as_ref()
                .and_then(|l| l.preferred_file.as_deref()),
        )
        .map_err(|e| e.message)?;
        crate::install_ownership::delete_open_file(&verified)?;
        Ok(resolved)
    })
    .await
    .map_err(|e| e.to_string())?
}
