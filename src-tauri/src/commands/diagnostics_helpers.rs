pub(super) fn build_health_report(state: &AppState) -> Result<HealthReport, String> {
    let store = lock_app_store(state)?;
    let repositories = store.list_repositories()?;
    let catalog = store.get_catalog()?;
    let configs = store.list_emulator_configs()?;
    let downloads = store.list_download_records()?;
    let torrent_downloads = store.list_torrent_downloads()?;
    let mut system_file_ids = HashSet::new();
    let mut system_files = Vec::new();
    let mut game_files = Vec::new();
    let platform_setup = setup_profiles::list_platform_setup_profiles()
        .into_iter()
        .map(|profile| {
            let emulator = build_setup_emulator_state(
                &store,
                &CatalogGameView {
                    id: String::new(),
                    source_id: String::new(),
                    repository_id: String::new(),
                    repository_name: String::new(),
                    platform: profile.platform.clone(),
                    title: profile.display_name.clone(),
                    description: None,
                    cover_image_url: None,
                    trailer_url: None,
                    artwork: None,
                    metadata: None,
                    content_mode: None,
                    setup_profile_id: Some(profile.id.clone()),
                    downloads: Vec::new(),
                    expected_extensions: profile.game_files.expected_extensions.clone(),
                    required_system_file_ids: Vec::new(),
                    launch: None,
                },
                Some(&profile),
            )?;
            let profile_files = profile
                .system_files
                .iter()
                .map(|requirement| {
                    inspect_profile_system_file(&store, &state.data_dir, &profile, requirement)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let required_missing = profile_files
                .iter()
                .filter(|item| item.required && item.status != "ready")
                .count();
            let ready = emulator.status == "ready" && required_missing == 0;
            Ok(HealthCheckItem {
                id: format!("profile:{}", profile.id),
                label: profile.display_name,
                status: if ready { "ready" } else { "missing" }.to_string(),
                message: Some(if ready {
                    "Profile setup is ready.".to_string()
                } else if emulator.status != "ready" {
                    emulator
                        .message
                        .unwrap_or_else(|| "Emulator is missing.".to_string())
                } else {
                    format!("{required_missing} required profile file(s) missing")
                }),
                action: Some(if ready {
                    "openProfileFolder".to_string()
                } else {
                    "configureProfile".to_string()
                }),
                path: emulator.executable_path,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let emulators = setup_profiles::mvp_platforms()
        .map(|profile| {
            let config = configs
                .iter()
                .find(|config| config.platform == profile.platform);
            let path = config.and_then(|config| config.exe_path.clone());
            let status = match path
                .as_deref()
                .map(|path| validate_emulator_status(Some(path)))
            {
                Some("valid") => "ready",
                Some("missing") => "missing",
                _ => "missing",
            };
            let executable_hint = profile
                .emulator
                .executable_name
                .clone()
                .or_else(|| profile.emulator.executable_candidates.first().cloned())
                .unwrap_or_else(|| format!("{} emulator", profile.platform.to_uppercase()));
            let platform_label = setup_profiles::platform_display_label(&profile.platform)
                .unwrap_or_else(|| profile.platform.to_uppercase());
            let emulator_name = setup_profiles::platform_emulator_name(&profile.platform)
                .unwrap_or_else(|| format!("{} emulator", profile.platform.to_uppercase()));
            HealthCheckItem {
                id: format!("emulator:{}", profile.platform),
                label: format!("{emulator_name} ({platform_label})"),
                status: status.to_string(),
                message: if status == "ready" {
                    Some(executable_hint.clone())
                } else {
                    Some(format!("Expected executable: {executable_hint}"))
                },
                action: Some(if status == "ready" {
                    "openEmulatorFolder".to_string()
                } else {
                    "reconfigureEmulator".to_string()
                }),
                path,
            }
        })
        .collect::<Vec<_>>();

    for game in &catalog {
        if let Some(download) = downloads
            .iter()
            .find(|download| download.subject_id == game.id)
        {
            let path = download.local_path.clone().unwrap_or_default();
            let game_status = check_game_file_health(&path, game);
            game_files.push(HealthCheckItem {
                id: format!("game:{}", game.id),
                label: game.title.clone(),
                status: game_status.0,
                message: game_status.1,
                action: Some("openGameFolder".to_string()),
                path: Some(path),
            });
        } else if let Some(torrent) = torrent_downloads
            .iter()
            .find(|download| download.game_id == game.id && download.status == "completed")
        {
            let game_status = check_game_file_health(&torrent.save_dir, game);
            game_files.push(HealthCheckItem {
                id: format!("game:{}", game.id),
                label: game.title.clone(),
                status: game_status.0,
                message: game_status.1,
                action: Some("openGameFolder".to_string()),
                path: Some(torrent.save_dir.clone()),
            });
        }

        let requirements = build_requirements_report(&store, &state.data_dir, game)?;
        for item in requirements.requirements {
            if !system_file_ids.insert(item.asset.id.clone()) {
                continue;
            }
            system_files.push(HealthCheckItem {
                id: format!("asset:{}", item.asset.id),
                label: item.asset.display_name,
                status: match item.status.as_str() {
                    "ready" if item.trusted => "ready",
                    "corrupt" => "corrupt",
                    "blocked" => "blocked",
                    "error" => "error",
                    _ => "missing",
                }
                .to_string(),
                message: item.message.or_else(|| item.target_path.clone()),
                action: Some(
                    match item.status.as_str() {
                        "corrupt" | "error" => "redownloadAsset",
                        "ready" if !item.trusted => "trustExecutable",
                        _ => "openTargetFolder",
                    }
                    .to_string(),
                ),
                path: item.target_path,
            });
        }
    }

    let repositories = repositories
        .into_iter()
        .map(|repository| HealthCheckItem {
            id: format!("repository:{}", repository.id),
            label: repository.name,
            status: "ready".to_string(),
            message: Some(format!(
                "{} games / {} system files / {}",
                repository.catalog_count, repository.system_file_count, repository.url
            )),
            action: Some("refreshRepository".to_string()),
            path: Some(repository.url),
        })
        .collect::<Vec<_>>();

    let active_downloads = torrent_downloads
        .iter()
        .filter(|download| matches!(download.status.as_str(), "resolving" | "downloading"))
        .count();
    let downloader = HealthCheckItem {
        id: "downloader:librqbit".to_string(),
        label: "Downloader session".to_string(),
        status: "ready".to_string(),
        message: Some(format!("{active_downloads} active torrent download(s)")),
        action: None,
        path: Some(
            download_root_for_app_state(state)?
                .to_string_lossy()
                .to_string(),
        ),
    };

    Ok(HealthReport {
        generated_at: Utc::now().to_rfc3339(),
        emulators,
        platform_setup,
        system_files,
        game_files,
        repositories,
        downloader,
    })
}

pub(super) fn check_game_file_health(
    path: &str,
    game: &CatalogGameView,
) -> (String, Option<String>) {
    if path.trim().is_empty() {
        return (
            "missing".to_string(),
            Some("No local path recorded.".to_string()),
        );
    }
    let path = Path::new(path);
    let preferred_file = game
        .launch
        .as_ref()
        .and_then(|launch| launch.preferred_file.as_deref());
    game_files::inspect_game_path(path, &game.expected_extensions, preferred_file)
}

pub(super) fn remove_path_if_allowed(
    data_dir: &Path,
    download_root: &Path,
    candidate: &Path,
) -> Result<(), String> {
    if !candidate.exists() {
        return Ok(());
    }
    let canonical_candidate = fs::canonicalize(candidate)
        .map_err(|error| format!("Failed to inspect {}: {error}", candidate.display()))?;
    let canonical_data_dir = fs::canonicalize(data_dir)
        .map_err(|error| format!("Failed to inspect app data directory: {error}"))?;
    let canonical_download_root = fs::canonicalize(download_root)
        .map_err(|error| format!("Failed to inspect download folder: {error}"))?;
    if !canonical_candidate.starts_with(&canonical_data_dir)
        && !canonical_candidate.starts_with(&canonical_download_root)
    {
        return Err(format!(
            "Refusing to delete files outside Fusion Launcher folders: {}",
            canonical_candidate.display()
        ));
    }
    if canonical_candidate.is_dir() {
        fs::remove_dir_all(&canonical_candidate)
    } else {
        fs::remove_file(&canonical_candidate)
    }
    .map_err(|error| {
        format!(
            "Failed to remove {}: {error}",
            canonical_candidate.display()
        )
    })
}

pub(super) fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    }
    Ok(())
}

pub(super) fn folder_path_for_open(path: &Path) -> Result<PathBuf, String> {
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }
    if path.is_file() {
        return path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("Game file has no parent directory: {}", path.display()));
    }
    Err(format!("Game path does not exist: {}", path.display()))
}
