pub(super) fn is_user_provided_game(game: &CatalogGameView) -> bool {
    game.content_mode.as_deref() == Some("user_provided")
        || game
            .downloads
            .iter()
            .any(|source| matches!(source, SourceUri::UserProvided { .. }))
}

pub(super) fn is_downloadable_game(game: &CatalogGameView) -> bool {
    !matches!(
        game.content_mode.as_deref(),
        Some("user_provided" | "metadata_only")
    ) && game.downloads.iter().any(|source| {
        matches!(
            source,
            SourceUri::Http { .. } | SourceUri::Bundled { .. } | SourceUri::Magnet { .. }
        )
    })
}

pub(super) fn inspect_game_download(
    store: &RepositoryStore,
    game: &CatalogGameView,
) -> Result<(bool, Option<String>, Option<String>), String> {
    let local_path = store
        .get_download(&game.id)?
        .and_then(|download| download.local_path);
    let local_path = match local_path {
        Some(local_path) => Some(local_path),
        None => store
            .get_torrent_download(&game.id)?
            .filter(|record| record.status == "completed")
            .map(|record| record.save_dir),
    };
    let Some(local_path) = local_path else {
        return Ok((false, None, None));
    };

    let profile = resolve_known_profile(game);
    let preferred_file = game
        .launch
        .as_ref()
        .and_then(|launch| launch.preferred_file.as_deref())
        .or_else(|| {
            profile
                .as_ref()
                .and_then(|profile| profile.launch.preferred_file.as_deref())
        });
    let expected_extensions = resolved_expected_extensions(game, profile.as_ref());
    let (status, message) =
        game_files::inspect_game_path(Path::new(&local_path), &expected_extensions, preferred_file);

    Ok((status == "ready", Some(status), message))
}

pub(super) fn build_library_status(
    store: &RepositoryStore,
    data_dir: &Path,
    game: &CatalogGameView,
) -> Result<LibraryGameStatus, String> {
    let requirements = build_requirements_report(store, data_dir, game)?;
    let download = store.get_torrent_download(&game.id)?;
    let game_file = inspect_game_download(store, game)?;
    let installed = game_file.1.is_some()
        || download
            .as_ref()
            .map(|record| record.status == "completed")
            .unwrap_or(false);
    let mut missing_requirements = Vec::new();

    for item in requirements.requirements {
        match item.status.as_str() {
            "ready" => {
                if !item.trusted {
                    missing_requirements
                        .push(format!("{} is not trusted", item.asset.display_name));
                }
            }
            "corrupt" => {
                missing_requirements.push(format!("{} is corrupt", item.asset.display_name));
            }
            "blocked" => {
                missing_requirements.push(item.message.unwrap_or_else(|| {
                    format!("{} cannot be installed yet", item.asset.display_name)
                }));
            }
            "error" => {
                missing_requirements.push(
                    item.message.unwrap_or_else(|| {
                        format!("{} installation failed", item.asset.display_name)
                    }),
                );
            }
            _ => {
                missing_requirements.push(format!("{} is not installed", item.asset.display_name));
            }
        }
    }
    if let (Some("missing" | "corrupt" | "error"), Some(message)) =
        (game_file.1.as_deref(), game_file.2)
    {
        missing_requirements.push(format!("Game file: {message}"));
    }

    Ok(LibraryGameStatus {
        game_id: game.id.clone(),
        installed,
        system_requirements_ready: missing_requirements.is_empty(),
        missing_requirements,
        download,
    })
}
