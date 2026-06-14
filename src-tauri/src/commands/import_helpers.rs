pub(super) struct BlockedAssetTarget {
    target_path: Option<String>,
    message: String,
}

pub(super) struct ImportedUserFile {
    status: &'static str,
    installed_path: String,
    sha256: String,
}

pub(super) struct ImportFileError {
    code: &'static str,
    sha256: Option<String>,
}

pub(super) fn import_file_error(code: &'static str) -> ImportFileError {
    ImportFileError { code, sha256: None }
}

pub(super) fn validate_import_source(source_path: &Path) -> Result<(), ImportFileError> {
    if source_path.as_os_str().is_empty() || !source_path.exists() {
        return Err(import_file_error("source_missing"));
    }
    if !source_path.is_file() {
        return Err(import_file_error("source_not_file"));
    }
    Ok(())
}

pub(super) fn copy_user_file_to_target(
    source_path: &Path,
    target: &Path,
    expected_sha256: Option<&str>,
) -> Result<ImportedUserFile, ImportFileError> {
    let installed_path = target.to_string_lossy().to_string();

    if target.exists() && target.is_file() {
        match hash_file(target) {
            Ok(actual_sha256)
                if expected_sha256
                    .map(|expected| actual_sha256.eq_ignore_ascii_case(expected))
                    .unwrap_or(true) =>
            {
                return Ok(ImportedUserFile {
                    status: "already_installed",
                    installed_path,
                    sha256: actual_sha256,
                });
            }
            Ok(_) => {}
            Err(_) => return Err(import_file_error("copy_failed")),
        }
    }

    validate_import_source(source_path)?;
    let source_sha256 = hash_file(source_path).map_err(|_| import_file_error("copy_failed"))?;
    if let Some(expected_sha256) = expected_sha256 {
        if !source_sha256.eq_ignore_ascii_case(expected_sha256) {
            return Err(ImportFileError {
                code: "checksum_mismatch",
                sha256: Some(source_sha256),
            });
        }
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|_| import_file_error("copy_failed"))?;
    }
    fs::copy(source_path, target).map_err(|_| import_file_error("copy_failed"))?;

    let installed_sha256 = hash_file(target).map_err(|_| import_file_error("copy_failed"))?;
    if let Some(expected_sha256) = expected_sha256 {
        if !installed_sha256.eq_ignore_ascii_case(expected_sha256) {
            return Err(ImportFileError {
                code: "checksum_mismatch",
                sha256: Some(installed_sha256),
            });
        }
    }

    Ok(ImportedUserFile {
        status: "installed",
        installed_path,
        sha256: installed_sha256,
    })
}

pub(super) fn resolve_asset_target(
    store: &RepositoryStore,
    data_dir: &Path,
    asset: &AssetView,
    source: &SourceUri,
) -> Result<PathBuf, BlockedAssetTarget> {
    let Some(hint) = asset.install_hint.as_ref() else {
        return Ok(destination_for_source(
            &data_dir.join("System"),
            &asset.platform,
            &asset.id,
            source,
            &asset.display_name,
        ));
    };

    match &hint.target {
        InstallTarget::AppSystem => {
            let Some(relative_path) = safe_relative_path_or_default(
                hint.relative_path.as_deref(),
                &file_name_for_source(source, &asset.display_name),
            ) else {
                return Err(blocked_asset(None, "System file install path is invalid."));
            };
            Ok(data_dir
                .join("System")
                .join(&asset.platform)
                .join(relative_path))
        }
        InstallTarget::EmulatorDir => {
            let config = store
                .get_emulator_config(&asset.platform)
                .map_err(|message| blocked_asset(None, message))?
                .ok_or_else(|| {
                    blocked_asset(
                        None,
                        format!(
                            "Configure the emulator for {} before installing {}.",
                            asset.platform, asset.display_name
                        ),
                    )
                })?;
            let exe_path = config.exe_path.as_deref().ok_or_else(|| {
                blocked_asset(
                    None,
                    format!(
                        "Configure the emulator for {} before installing {}.",
                        asset.platform, asset.display_name
                    ),
                )
            })?;
            let emulator_path = Path::new(exe_path);
            if validate_emulator_status(Some(exe_path)) != "valid" {
                return Err(blocked_asset(
                    Some(exe_path.to_string()),
                    format!(
                        "Emulator path for {} is not valid: {}",
                        asset.platform, exe_path
                    ),
                ));
            }
            let parent = emulator_path.parent().ok_or_else(|| {
                blocked_asset(
                    Some(exe_path.to_string()),
                    format!("Emulator path has no parent directory: {exe_path}"),
                )
            })?;
            let Some(relative_path) = safe_relative_path_or_default(
                hint.relative_path.as_deref(),
                &file_name_for_source(source, &asset.display_name),
            ) else {
                return Err(blocked_asset(None, "System file install path is invalid."));
            };
            Ok(parent.join(relative_path))
        }
        InstallTarget::UserSelected => Err(blocked_asset(
            None,
            format!(
                "{} requires a user-selected install path, which is not supported in v1.",
                asset.display_name
            ),
        )),
    }
}

pub(super) fn inspect_asset_installation(
    store: &RepositoryStore,
    data_dir: &Path,
    asset: &AssetView,
    download: Option<&DownloadRecord>,
) -> Result<crate::schema::AssetInstallation, String> {
    let source = asset.sources.first();
    let expected_sha256 = expected_asset_sha256(asset);
    let target = match download
        .filter(|record| is_download_ready_status(&record.status))
        .and_then(|record| record.local_path.as_ref())
        .map(PathBuf::from)
    {
        Some(path) => Ok(path),
        None => {
            if let Some(source) = source {
                resolve_asset_target(store, data_dir, asset, source)
            } else {
                Err(blocked_asset(
                    None,
                    format!("{} has no sources.", asset.display_name),
                ))
            }
        }
    };

    let target = match target {
        Ok(path) => path,
        Err(blocked) => {
            return store.record_asset_installation(
                &asset.id,
                blocked.target_path.as_deref(),
                "blocked",
                None,
                Some(&blocked.message),
            );
        }
    };
    let target_path = target.to_string_lossy().to_string();

    if !target.exists() {
        return store.record_asset_installation(
            &asset.id,
            Some(&target_path),
            "missing",
            None,
            None,
        );
    }

    if !target.is_file() {
        return store.record_asset_installation(
            &asset.id,
            Some(&target_path),
            "error",
            None,
            Some("System file target is not a file."),
        );
    }

    if let Some(expected_sha256) = expected_sha256 {
        let actual_sha256 = hash_file(&target)?;
        if actual_sha256.eq_ignore_ascii_case(expected_sha256) {
            store.record_asset_installation(
                &asset.id,
                Some(&target_path),
                "ready",
                Some(&actual_sha256),
                None,
            )
        } else {
            store.record_asset_installation(
                &asset.id,
                Some(&target_path),
                "corrupt",
                Some(&actual_sha256),
                Some(&format!(
                    "SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
                )),
            )
        }
    } else {
        store.record_asset_installation(&asset.id, Some(&target_path), "ready", None, None)
    }
}

pub(super) fn import_asset_file_into_store(
    store: &RepositoryStore,
    data_dir: &Path,
    asset_id: &str,
    source_path: &Path,
) -> ImportAssetFileReport {
    let asset = match store.get_asset(asset_id) {
        Ok(Some(asset)) => asset,
        Ok(None) => return import_asset_error("", "unknown_asset"),
        Err(_) => return import_asset_error("", "store_failed"),
    };

    let Some(source) = asset.sources.first() else {
        return import_asset_error("", "unsupported_target");
    };
    if !matches!(source, SourceUri::UserProvided { .. }) {
        return import_asset_error("", "unsupported_target");
    }

    let target = match resolve_asset_target(store, data_dir, &asset, source) {
        Ok(target) => target,
        Err(blocked) => {
            return import_asset_error(
                blocked.target_path.unwrap_or_default(),
                "unsupported_target",
            )
        }
    };
    let installed_path = target.to_string_lossy().to_string();
    let checksum = expected_asset_sha256(&asset);

    match copy_user_file_to_target(source_path, &target, checksum) {
        Ok(imported) => {
            if record_imported_asset(
                store,
                &asset.id,
                &imported.installed_path,
                Some(&imported.sha256),
            )
            .is_err()
            {
                return import_asset_error(imported.installed_path, "store_failed");
            }
            ImportAssetFileReport {
                status: imported.status.to_string(),
                installed_path: imported.installed_path,
                error_code: None,
            }
        }
        Err(error) => {
            if error.code == "checksum_mismatch" {
                let expected_sha256 = checksum.unwrap_or("");
                let actual_sha256 = error.sha256.as_deref().unwrap_or("");
                let _ = store.record_asset_installation(
                    &asset.id,
                    Some(&installed_path),
                    "error",
                    error.sha256.as_deref(),
                    Some(&format!(
                        "SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
                    )),
                );
            }
            import_asset_error(installed_path, error.code)
        }
    }
}

pub(super) fn import_game_file_into_store(
    store: &RepositoryStore,
    download_root: &Path,
    game_id: &str,
    source_path: &Path,
) -> ImportGameFileReport {
    let game = match store.get_game(game_id) {
        Ok(Some(game)) => game,
        Ok(None) => return import_game_error(game_id, "", "unknown_game", None),
        Err(_) => return import_game_error(game_id, "", "store_failed", None),
    };

    if game.content_mode.as_deref() == Some("metadata_only") {
        return import_game_error(&game.id, "", "unsupported_target", None);
    }
    let user_source = game
        .downloads
        .iter()
        .find(|source| matches!(source, SourceUri::UserProvided { .. }));
    if game.content_mode.as_deref() != Some("user_provided") && user_source.is_none() {
        return import_game_error(&game.id, "", "unsupported_target", None);
    }
    let expected_sha256 = user_source.and_then(|source| match source {
        SourceUri::UserProvided { sha256, .. } => sha256.as_deref(),
        _ => None,
    });

    if let Err(error) = validate_import_source(source_path) {
        return import_game_error(&game.id, "", error.code, None);
    }
    let profile = resolve_known_profile(&game);
    let expected_extensions = resolved_expected_extensions(&game, profile.as_ref());
    if !source_matches_expected_extension(source_path, &expected_extensions) {
        return import_game_error(&game.id, "", "wrong_extension", None);
    }

    let target = imported_game_target(download_root, &game, source_path);
    let installed_path = target.to_string_lossy().to_string();
    match copy_user_file_to_target(source_path, &target, expected_sha256) {
        Ok(imported) => {
            let total_bytes = file_size(Path::new(&imported.installed_path));
            if store
                .record_direct_game_download_completed(
                    &game.id,
                    "user_import",
                    &imported.installed_path,
                    &imported.sha256,
                    total_bytes,
                )
                .is_err()
            {
                return import_game_error(
                    &game.id,
                    &imported.installed_path,
                    "store_failed",
                    Some(imported.sha256),
                );
            }
            if store
                .upsert_rom_hash_sha256(&game.id, &imported.sha256, total_bytes)
                .is_err()
            {
                return import_game_error(
                    &game.id,
                    &imported.installed_path,
                    "store_failed",
                    Some(imported.sha256),
                );
            }
            ImportGameFileReport {
                status: imported.status.to_string(),
                game_id: game.id,
                installed_path: imported.installed_path,
                sha256: Some(imported.sha256),
                error_code: None,
            }
        }
        Err(error) => import_game_error(&game.id, &installed_path, error.code, error.sha256),
    }
}

pub(super) fn imported_game_target(
    download_root: &Path,
    game: &CatalogGameView,
    source_path: &Path,
) -> PathBuf {
    let file_name = source_path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(crate::downloads::safe_segment)
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or_else(|| crate::downloads::safe_segment(&game.title));
    download_root
        .join(crate::downloads::safe_segment(&game.platform))
        .join(crate::downloads::safe_segment(&game.id))
        .join(file_name)
}

pub(super) fn source_matches_expected_extension(
    source_path: &Path,
    expected_extensions: &[String],
) -> bool {
    let Some(extension) = source_path
        .extension()
        .and_then(|extension| extension.to_str())
    else {
        return false;
    };
    let extension = format!(".{}", extension.to_ascii_lowercase());
    expected_extensions
        .iter()
        .any(|expected| expected.eq_ignore_ascii_case(&extension))
}

pub(super) fn import_game_error(
    game_id: impl Into<String>,
    installed_path: impl Into<String>,
    error_code: impl Into<String>,
    sha256: Option<String>,
) -> ImportGameFileReport {
    ImportGameFileReport {
        status: "error".to_string(),
        game_id: game_id.into(),
        installed_path: installed_path.into(),
        sha256,
        error_code: Some(error_code.into()),
    }
}

pub(super) fn record_imported_asset(
    store: &RepositoryStore,
    asset_id: &str,
    installed_path: &str,
    sha256: Option<&str>,
) -> Result<(), String> {
    store.record_asset_installation(asset_id, Some(installed_path), "ready", sha256, None)?;
    store.record_imported_asset_download(asset_id, installed_path, sha256)?;
    Ok(())
}

pub(super) fn import_asset_error(
    installed_path: impl Into<String>,
    error_code: impl Into<String>,
) -> ImportAssetFileReport {
    ImportAssetFileReport {
        status: "error".to_string(),
        installed_path: installed_path.into(),
        error_code: Some(error_code.into()),
    }
}

pub(super) fn expected_asset_sha256(asset: &AssetView) -> Option<&str> {
    asset.sources.iter().find_map(|source| match source {
        SourceUri::Http { sha256, .. } => Some(sha256.as_str()),
        SourceUri::Bundled { sha256, .. } => Some(sha256.as_str()),
        SourceUri::Magnet { .. } => None,
        SourceUri::UserProvided { sha256, .. } => sha256.as_deref(),
    })
}

pub(super) fn is_download_ready_status(status: &str) -> bool {
    matches!(status, "ready" | "completed")
}

pub(super) fn source_size_bytes(source: &SourceUri) -> Option<u64> {
    match source {
        SourceUri::Http { size_bytes, .. }
        | SourceUri::Bundled { size_bytes, .. }
        | SourceUri::Magnet { size_bytes, .. }
        | SourceUri::UserProvided { size_bytes, .. } => *size_bytes,
    }
}

pub(super) fn direct_source_kind(source: &SourceUri) -> &'static str {
    match source {
        SourceUri::Bundled { .. } => "bundled",
        SourceUri::Http { .. } => "http",
        SourceUri::Magnet { .. } => "magnet",
        SourceUri::UserProvided { .. } => "user_provided",
    }
}

pub(super) fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

pub(super) fn blocked_asset(
    target_path: Option<String>,
    message: impl Into<String>,
) -> BlockedAssetTarget {
    BlockedAssetTarget {
        target_path,
        message: message.into(),
    }
}

pub(super) fn safe_relative_path_or_default(
    input: Option<&str>,
    fallback_file_name: &str,
) -> Option<PathBuf> {
    let raw = input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_file_name);
    let path = Path::new(raw);
    if path.is_absolute() {
        return None;
    }

    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => sanitized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if sanitized.as_os_str().is_empty() {
        None
    } else {
        Some(sanitized)
    }
}
