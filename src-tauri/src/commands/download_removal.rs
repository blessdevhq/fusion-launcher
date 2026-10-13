#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalReport {
    pub changed: bool,
    pub preserved_paths: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRemovedEvent {
    game_id: String,
    attempt_id: Option<String>,
}

#[tauri::command]
pub async fn remove_game(
    game_id: String,
    delete_files: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<RemovalReport, String> {
    remove_download_internal(&game_id, delete_files, &state, &app, "game_removed").await
}
#[tauri::command]
pub async fn remove_download(
    download_id: String,
    delete_files: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<RemovalReport, String> {
    remove_download_internal(&download_id, delete_files, &state, &app, "download_removed").await
}

async fn remove_download_internal(
    id: &str,
    delete_files: bool,
    state: &AppState,
    app: &AppHandle,
    event: &str,
) -> Result<RemovalReport, String> {
    let ticket = state.direct_downloads.begin_delete(id)?;
    // Inspect transport only under the reserved ID. Read final paths AFTER stop.
    let is_torrent = lock_app_store(state)?
        .get_torrent_download(id)?
        .is_some_and(|r| !r.magnet_uri.starts_with("direct:"));
    if is_torrent {
        if let Some(manager) = state.torrents.as_ref() {
            manager.stop_download(id).await?;
        }
    }
    ticket.wait_owner(Duration::from_secs(20)).await?;
    let (download, torrent, ownership) = {
        let store = lock_app_store(state)?;
        (
            store.get_download(id)?,
            store.get_torrent_download(id)?,
            crate::download_installation::read_all(&store, id)?,
        )
    };
    let attempt = torrent.as_ref().and_then(|r| r.attempt_id.clone());
    let mut preserved = Vec::new();
    let mut retained = Vec::new();
    let candidates = download
        .as_ref()
        .and_then(|r| r.local_path.as_deref())
        .into_iter()
        .chain(torrent.as_ref().map(|r| r.save_dir.as_str()))
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if delete_files {
        // Every historical manifest belongs to a finished attempt of this
        // reserved subject; none is inferred from a path or archive marker.
        for owned in &ownership {
            let leftovers = crate::download_installation::cleanup(owned.clone()).await?;
            if !leftovers.is_empty() {
                retained.push(owned.clone());
            }
            preserved.extend(leftovers);
        }
        for candidate in &candidates {
            if !ownership
                .iter()
                .any(|owned| candidate.starts_with(owned.root()))
                && fs::symlink_metadata(candidate).is_ok()
            {
                preserved.push(candidate.to_string_lossy().into());
            }
        }
    }
    preserved.sort();
    preserved.dedup();
    // Keep rows/authority on an OS sharing/access error so cleanup can be retried.
    if preserved
        .iter()
        .any(|s| s.contains("os error 32") || s.contains("os error 5"))
    {
        return Err(format!(
            "Files remain locked; removal can be retried: {}",
            preserved.join("; ")
        ));
    }
    let changed = download.is_some() || torrent.is_some() || !ownership.is_empty();
    {
        let store = lock_app_store(state)?;
        let asset = download.as_ref().is_some_and(|r| r.subject_type == "asset")
            || torrent.as_ref().and_then(|r| r.subject_type.as_deref()) == Some("asset");
        // Persist history before dropping current authority so a failed write
        // cannot lose the manifests required for a retry.
        crate::download_installation::replace_history(&store, id, &retained)?;
        store.delete_attempt_state(id, asset)?;
    }
    if is_torrent {
        if let Some(manager) = state.torrents.as_ref() {
            manager.forget_download(id).await;
        }
    }
    let _ = app.emit(
        "download:removed",
        DownloadRemovedEvent {
            game_id: id.into(),
            attempt_id: attempt,
        },
    );
    logging::log_event(&state.data_dir, event, &[("download_id", id)]);
    drop(ticket);
    if let Some(manager) = state.torrents.as_ref() {
        manager.process_next_queued_download().await;
    }
    Ok(RemovalReport {
        changed,
        preserved_paths: preserved,
    })
}
