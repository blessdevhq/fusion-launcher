pub(super) fn build_diagnostics_bundle(state: &AppState) -> Result<DiagnosticsBundle, String> {
    let health = build_health_report(state)?;
    let downloads = state.torrents.list_downloads()?;
    let log_path = logging::log_file_path(&state.data_dir);
    Ok(DiagnosticsBundle {
        generated_at: Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        data_dir: state.data_dir.to_string_lossy().to_string(),
        log_path: log_path.to_string_lossy().to_string(),
        health,
        downloads,
        logs: logging::tail_log(&state.data_dir, 500),
    })
}

pub(crate) fn build_requirements_report(
    store: &RepositoryStore,
    data_dir: &Path,
    game: &CatalogGameView,
) -> Result<RequirementsReport, String> {
    let game_downloaded = inspect_game_download(store, game)?.0;
    let assets = store.get_assets(&game.required_system_file_ids)?;
    let mut requirements = Vec::new();

    for asset in assets {
        let download = store.get_download(&asset.id)?;
        let trusted = store.get_trusted_executable(&asset.id)?;
        let installation = inspect_asset_installation(store, data_dir, &asset, download.as_ref())?;
        let downloaded = installation.status == "ready";
        let checksum = expected_asset_sha256(&asset).map(ToString::to_string);
        let trusted_ok = if asset.executable {
            trusted.is_some()
        } else {
            true
        };
        requirements.push(RequirementItem {
            asset,
            status: installation.status,
            downloaded,
            trusted: trusted_ok,
            local_path: download.and_then(|record| record.local_path),
            target_path: installation.target_path,
            checksum,
            sha256: installation.sha256,
            message: installation.message,
        });
    }

    let ready = game_downloaded
        && requirements
            .iter()
            .all(|item| item.downloaded && item.trusted);
    Ok(RequirementsReport {
        game_id: game.id.clone(),
        ready,
        game_downloaded,
        requirements,
    })
}
