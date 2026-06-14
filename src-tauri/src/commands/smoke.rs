pub(crate) fn run_package_smoke(data_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("Failed to create package smoke data dir: {error}"))?;
    let db_path = data_dir.join("fusion-launcher.db");
    let mut store = RepositoryStore::open(&db_path)?;

    let demo_repo = builtin_demo::repository_schema()?;
    builtin_demo::verify_embedded_assets(&demo_repo)?;
    store.store_repository(builtin_demo::BUILTIN_DEMO_REPOSITORY_URL, &demo_repo)?;
    let demo_game = store
        .get_game("fusion-launcher-demo::fusion_launcher_nes_smoke")?
        .ok_or_else(|| "Built-in demo game was not stored.".to_string())?;
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Failed to resolve current executable: {error}"))?;
    let current_exe_string = current_exe.to_string_lossy().to_string();
    store.upsert_profile_emulator_config(
        "nes-mesen",
        "nes",
        Some(&current_exe_string),
        "valid",
        Some("package-smoke"),
        Some("{game_path}"),
    )?;

    let Some(SourceUri::Bundled { path, sha256, .. }) = demo_repo.catalog[0].downloads.first()
    else {
        return Err("Built-in demo game must use a bundled source.".to_string());
    };
    let bytes = builtin_demo::asset_bytes(path)
        .ok_or_else(|| format!("Built-in demo asset is missing: {path}"))?;
    let demo_target = data_dir
        .join("Games")
        .join("nes")
        .join("fusion-launcher-demo__fusion_launcher_nes_smoke")
        .join("fusion-launcher-smoke.nes");
    if let Some(parent) = demo_target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create demo game folder: {error}"))?;
    }
    fs::write(&demo_target, bytes)
        .map_err(|error| format!("Failed to write bundled demo asset: {error}"))?;
    store.record_direct_game_download_completed(
        &demo_game.id,
        "bundled",
        &demo_target.to_string_lossy(),
        sha256,
        bytes.len() as u64,
    )?;
    let demo_state = build_game_setup_state(&store, data_dir, &demo_game)?;
    if demo_state.launch.status != "ready" {
        return Err(format!(
            "Built-in demo setup did not become ready: {:?}",
            demo_state.launch.blockers
        ));
    }

    let switch_repo = package_smoke_switch_repo();
    store.store_repository("package-smoke://switch", &switch_repo)?;
    let switch_profile = setup_profiles::get_platform_setup_profile("switch-manual")
        .ok_or_else(|| "switch-manual profile is missing.".to_string())?;
    store.upsert_profile_emulator_config(
        "switch-manual",
        "switch",
        Some(&current_exe_string),
        "valid",
        Some("package-smoke"),
        Some("{game_path}"),
    )?;
    let keys_source = data_dir.join("fixtures").join("prod.keys");
    let xci_source = data_dir.join("fixtures").join("star-orbit.xci");
    fs::create_dir_all(keys_source.parent().unwrap())
        .map_err(|error| format!("Failed to create package smoke fixtures: {error}"))?;
    fs::write(&keys_source, b"package-smoke-keys")
        .map_err(|error| format!("Failed to write fake keys fixture: {error}"))?;
    fs::write(&xci_source, b"package-smoke-xci")
        .map_err(|error| format!("Failed to write fake game fixture: {error}"))?;
    let keys_requirement = switch_profile
        .system_files
        .iter()
        .find(|requirement| requirement.id == "switch-prod-keys")
        .ok_or_else(|| "switch-manual keys requirement is missing.".to_string())?;
    let key_import = import_profile_system_file_into_store(
        &store,
        data_dir,
        &switch_profile,
        keys_requirement,
        &keys_source,
    )?;
    if key_import.status == "error" {
        return Err(format!(
            "Profile system file import failed: {:?}",
            key_import.error_code
        ));
    }

    let switch_game = store
        .get_game("package-smoke::star-orbit")
        .map_err(|error| format!("Failed to read package smoke game: {error}"))?
        .ok_or_else(|| "Package smoke switch game was not stored.".to_string())?;
    let game_import = import_game_file_into_store(
        &store,
        &data_dir.join("Games"),
        &switch_game.id,
        &xci_source,
    );
    if game_import.status == "error" {
        return Err(format!(
            "Profile game file import failed: {:?}",
            game_import.error_code
        ));
    }
    let switch_state = build_game_setup_state(&store, data_dir, &switch_game)?;
    if switch_state.launch.status != "ready" {
        return Err(format!(
            "Switch manual setup did not become ready: {:?}",
            switch_state.launch.blockers
        ));
    }

    drop(store);
    let store = RepositoryStore::open(&db_path)?;
    let persisted_game = store
        .get_game("package-smoke::star-orbit")?
        .ok_or_else(|| "Package smoke switch game was not persisted.".to_string())?;
    let persisted_state = build_game_setup_state(&store, data_dir, &persisted_game)?;
    if persisted_state.launch.status != "ready" {
        return Err(format!(
            "Package smoke state was not persisted: {:?}",
            persisted_state.launch.blockers
        ));
    }

    Ok(())
}

fn package_smoke_switch_repo() -> RepositorySchema {
    RepositorySchema {
        metadata: RepositoryMetadata {
            id: "package-smoke".to_string(),
            name: "Package Smoke Repository".to_string(),
            version: "1".to_string(),
            schema_version: 3,
            maintainer: Some("Fusion Launcher".to_string()),
            homepage_url: None,
            license: Some("Synthetic smoke metadata only".to_string()),
            trust_level: Some("official".to_string()),
            content_hash: None,
            updated_at: None,
        },
        system_files: vec![],
        catalog: vec![RepositoryGame {
            id: "star-orbit".to_string(),
            platform: "switch".to_string(),
            title: "Star Orbit Package Smoke".to_string(),
            description: Some("Synthetic user-provided package smoke entry.".to_string()),
            cover_image_url: None,
            trailer_url: None,
            artwork: None,
            metadata: None,
            content_mode: Some("user_provided".to_string()),
            setup_profile_id: Some("switch-manual".to_string()),
            downloads: vec![SourceUri::UserProvided {
                instructions: Some("Import a local package smoke fixture.".to_string()),
                sha256: None,
                size_bytes: None,
            }],
            expected_extensions: vec![],
            required_system_file_ids: vec![],
            launch: None,
        }],
    }
}
