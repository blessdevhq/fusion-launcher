pub(crate) fn build_game_setup_state(
    store: &RepositoryStore,
    data_dir: &Path,
    game: &CatalogGameView,
) -> Result<GameSetupState, String> {
    let profile = resolve_known_profile(game);
    let unsupported_profile_id = match game.setup_profile_id.as_deref() {
        Some(profile_id) if profile.is_none() => Some(profile_id.to_string()),
        _ => None,
    };

    let repository_requirements = build_requirements_report(store, data_dir, game)?.requirements;
    let expected_extensions = resolved_expected_extensions(game, profile.as_ref());
    let preferred_file = game
        .launch
        .as_ref()
        .and_then(|launch| launch.preferred_file.as_deref())
        .or_else(|| {
            profile
                .as_ref()
                .and_then(|profile| profile.launch.preferred_file.as_deref())
        });
    let game_file = inspect_game_setup_file(store, game, &expected_extensions, preferred_file)?;
    let emulator = build_setup_emulator_state(store, game, profile.as_ref())?;
    let system_files = if let Some(profile) = profile.as_ref() {
        profile
            .system_files
            .iter()
            .map(|requirement| inspect_profile_system_file(store, data_dir, profile, requirement))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };

    let mut blockers = Vec::new();
    if let Some(profile_id) = unsupported_profile_id.as_ref() {
        blockers.push(format!("Unsupported setup profile: {profile_id}"));
    }
    if emulator.status != "ready" {
        blockers.push(
            emulator
                .message
                .clone()
                .unwrap_or_else(|| format!("Configure {}", emulator.emulator_name)),
        );
    }
    for item in &system_files {
        if item.required && item.status != "ready" {
            blockers.push(format!("Import {}", item.label));
        }
    }
    for item in &repository_requirements {
        if item.status != "ready" || !item.trusted {
            blockers.push(format!("Install {}", item.asset.display_name));
        }
    }
    if game_file.status != "ready" {
        blockers.push(
            game_file
                .message
                .clone()
                .unwrap_or_else(|| "Game file is missing.".to_string()),
        );
    }

    let launch = GameSetupLaunchState {
        status: if blockers.is_empty() {
            "ready".to_string()
        } else {
            "blocked".to_string()
        },
        blockers,
    };
    let primary_action =
        derive_primary_setup_action(game, &emulator, &system_files, &game_file, &launch);

    Ok(GameSetupState {
        game_id: game.id.clone(),
        profile_id: profile
            .as_ref()
            .map(|profile| profile.id.clone())
            .or_else(|| game.setup_profile_id.clone()),
        profile_display_name: profile.as_ref().map(|profile| profile.display_name.clone()),
        unsupported_profile_id,
        emulator,
        system_files,
        repository_requirements,
        game_file,
        launch,
        primary_action,
    })
}

pub(crate) fn resolved_expected_extensions(
    game: &CatalogGameView,
    profile: Option<&PlatformSetupProfile>,
) -> Vec<String> {
    profile
        .map(|profile| profile.game_files.expected_extensions.clone())
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| game.expected_extensions.clone())
}

pub(crate) fn resolve_known_profile(game: &CatalogGameView) -> Option<PlatformSetupProfile> {
    game.setup_profile_id
        .as_deref()
        .and_then(setup_profiles::get_platform_setup_profile)
        .or_else(|| setup_profiles::get_default_platform_setup_profile(&game.platform))
}

pub(super) fn build_setup_emulator_state(
    store: &RepositoryStore,
    game: &CatalogGameView,
    profile: Option<&PlatformSetupProfile>,
) -> Result<GameSetupEmulatorState, String> {
    if let Some(profile) = profile {
        let profile_config = store.get_profile_emulator_config(&profile.id)?;
        let exe_path = profile_config
            .as_ref()
            .and_then(|config| config.exe_path.clone());
        let status = validate_emulator_status(exe_path.as_deref());
        let ready = status == "valid";
        return Ok(GameSetupEmulatorState {
            status: if ready {
                "ready".to_string()
            } else if profile.emulator.install_mode == "manual" {
                "manual_required".to_string()
            } else {
                "missing".to_string()
            },
            profile_id: Some(profile.id.clone()),
            platform: profile.platform.clone(),
            emulator_name: profile.emulator.emulator_name.clone(),
            install_mode: profile.emulator.install_mode.clone(),
            executable_path: exe_path,
            message: if ready {
                None
            } else if profile.emulator.install_mode == "downloadable" {
                Some(format!("Install {}", profile.emulator.emulator_name))
            } else {
                Some(format!("Select {}", profile.emulator.emulator_name))
            },
        });
    }

    let config = store.get_emulator_config(&game.platform)?;
    let exe_path = config.as_ref().and_then(|config| config.exe_path.clone());
    let status = validate_emulator_status(exe_path.as_deref());
    Ok(GameSetupEmulatorState {
        status: if status == "valid" {
            "ready".to_string()
        } else {
            "manual_required".to_string()
        },
        profile_id: None,
        platform: game.platform.clone(),
        emulator_name: setup_profiles::platform_emulator_name(&game.platform)
            .unwrap_or_else(|| format!("{} emulator", game.platform.to_uppercase())),
        install_mode: "manual".to_string(),
        executable_path: exe_path,
        message: if status == "valid" {
            None
        } else {
            Some(format!(
                "Configure {} emulator",
                game.platform.to_uppercase()
            ))
        },
    })
}

pub(super) fn inspect_game_setup_file(
    store: &RepositoryStore,
    game: &CatalogGameView,
    expected_extensions: &[String],
    preferred_file: Option<&str>,
) -> Result<GameSetupGameFileState, String> {
    let local_path = store
        .get_download(&game.id)?
        .filter(|download| matches!(download.status.as_str(), "ready" | "completed"))
        .and_then(|download| download.local_path)
        .or_else(|| {
            store
                .get_torrent_download(&game.id)
                .ok()
                .flatten()
                .filter(|record| record.status == "completed")
                .map(|record| record.save_dir)
        });
    let Some(local_path) = local_path else {
        return Ok(GameSetupGameFileState {
            status: "missing".to_string(),
            installed_path: None,
            expected_extensions: expected_extensions.to_vec(),
            allow_directory: true,
            message: Some("Game file is missing.".to_string()),
        });
    };

    let (status, message) =
        game_files::inspect_game_path(Path::new(&local_path), expected_extensions, preferred_file);
    Ok(GameSetupGameFileState {
        status: if status == "ready" {
            "ready".to_string()
        } else {
            "invalid".to_string()
        },
        installed_path: Some(local_path),
        expected_extensions: expected_extensions.to_vec(),
        allow_directory: true,
        message,
    })
}

pub(super) fn inspect_profile_system_file(
    store: &RepositoryStore,
    data_dir: &Path,
    profile: &PlatformSetupProfile,
    requirement: &ProfileSystemFileRequirement,
) -> Result<GameSetupSystemFileState, String> {
    let import = store.get_profile_system_file_import(&profile.id, &requirement.id)?;
    let target = import
        .as_ref()
        .and_then(|import| import.target_path.clone())
        .unwrap_or_else(|| {
            profile_system_file_target(data_dir, profile, requirement)
                .to_string_lossy()
                .to_string()
        });
    let path = Path::new(&target);
    let mut status = "missing".to_string();
    let mut message = requirement.notes.clone();

    if path.exists() && path.is_file() {
        if !profile_system_file_matches_extension(path, requirement) {
            status = "corrupt".to_string();
            message = Some(format!(
                "{} has an unsupported extension. Expected: {}",
                requirement.label,
                requirement.extensions.join(", ")
            ));
        } else {
            let actual = hash_file(path)?;
            if let Some(expected) = requirement.checksum.as_deref() {
                if !actual.eq_ignore_ascii_case(expected) {
                    status = "corrupt".to_string();
                    message = Some(format!(
                        "SHA-256 mismatch: expected {expected}, got {actual}"
                    ));
                } else {
                    status = "ready".to_string();
                    message = Some(target.clone());
                }
            } else {
                status = "ready".to_string();
                message = Some(target.clone());
            }
        }
    }

    Ok(GameSetupSystemFileState {
        id: requirement.id.clone(),
        label: requirement.label.clone(),
        asset_kind: requirement.asset_kind.clone(),
        required: requirement.required,
        status,
        installed_path: if path.exists() { Some(target) } else { None },
        expected_extensions: requirement.extensions.clone(),
        checksum: requirement.checksum.clone(),
        message,
    })
}

pub(super) fn derive_primary_setup_action(
    game: &CatalogGameView,
    emulator: &GameSetupEmulatorState,
    system_files: &[GameSetupSystemFileState],
    game_file: &GameSetupGameFileState,
    launch: &GameSetupLaunchState,
) -> String {
    if launch.status == "ready" {
        return "play".to_string();
    }
    if game_file.status != "ready" && is_user_provided_game(game) {
        return "import_game".to_string();
    }
    if game_file.status != "ready" && is_downloadable_game(game) {
        return "download".to_string();
    }
    if emulator.status != "ready"
        || system_files
            .iter()
            .any(|item| item.required && item.status != "ready")
    {
        return "setup".to_string();
    }
    "details".to_string()
}

pub(super) fn import_profile_system_file_into_store(
    store: &RepositoryStore,
    data_dir: &Path,
    profile: &PlatformSetupProfile,
    requirement: &ProfileSystemFileRequirement,
    source_path: &Path,
) -> Result<ImportAssetFileReport, String> {
    if requirement.source_mode != "user_provided" {
        return Ok(import_asset_error("", "unsupported_target"));
    }
    if let Err(error) = validate_import_source(source_path) {
        return Ok(import_asset_error("", error.code));
    }
    if !profile_system_file_matches_extension(source_path, requirement) {
        return Ok(import_asset_error("", "wrong_extension"));
    }

    let target = profile_system_file_target(data_dir, profile, requirement);
    let installed_path = target.to_string_lossy().to_string();
    match copy_user_file_to_target(source_path, &target, requirement.checksum.as_deref()) {
        Ok(imported) => {
            store.record_profile_system_file_import(
                &profile.id,
                &requirement.id,
                Some(&imported.installed_path),
                "ready",
                Some(&imported.sha256),
                None,
            )?;
            Ok(ImportAssetFileReport {
                status: imported.status.to_string(),
                installed_path: imported.installed_path,
                error_code: None,
            })
        }
        Err(error) => {
            let _ = store.record_profile_system_file_import(
                &profile.id,
                &requirement.id,
                Some(&installed_path),
                "error",
                error.sha256.as_deref(),
                Some(error.code),
            );
            Ok(import_asset_error(installed_path, error.code))
        }
    }
}

pub(super) fn profile_system_file_target(
    data_dir: &Path,
    profile: &PlatformSetupProfile,
    requirement: &ProfileSystemFileRequirement,
) -> PathBuf {
    let target = requirement
        .target_name
        .as_deref()
        .and_then(|target_name| safe_relative_path_or_default(Some(target_name), &requirement.id))
        .unwrap_or_else(|| {
            PathBuf::from(format!(
                "{}{}",
                crate::downloads::safe_segment(&requirement.id),
                requirement
                    .extensions
                    .first()
                    .cloned()
                    .unwrap_or_else(|| ".bin".to_string())
            ))
        });
    data_dir
        .join("System")
        .join(crate::downloads::safe_segment(&profile.platform))
        .join(target)
}

pub(super) fn profile_system_file_matches_extension(
    source_path: &Path,
    requirement: &ProfileSystemFileRequirement,
) -> bool {
    if requirement.extensions.is_empty() {
        return true;
    }
    source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{}", extension.to_lowercase()))
        .map(|extension| {
            requirement
                .extensions
                .iter()
                .any(|expected| expected.eq_ignore_ascii_case(&extension))
        })
        .unwrap_or(false)
}
