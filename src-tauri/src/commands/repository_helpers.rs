pub(super) fn validate_emulator_status(exe_path: Option<&str>) -> &'static str {
    let Some(exe_path) = exe_path.map(str::trim).filter(|value| !value.is_empty()) else {
        return "invalid";
    };
    let path = Path::new(exe_path);
    if !path.exists() {
        return "missing";
    }
    if !path.is_file() {
        return "invalid";
    }
    if cfg!(windows)
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| !extension.eq_ignore_ascii_case("exe"))
            .unwrap_or(true)
    {
        return "invalid";
    }

    "valid"
}

pub(super) fn normalize_launch_args_template(value: Option<String>) -> Option<String> {
    value
        .map(|template| template.trim().to_string())
        .filter(|template| !template.is_empty())
}

pub(super) async fn fetch_repository_schema(
    url: &str,
    allow_dev_http: bool,
) -> Result<RepositorySchema, String> {
    let parsed = validate_repository_url(url, allow_dev_http)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| format!("Failed to initialize repository client: {error}"))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| format!("Failed to fetch repository: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Repository returned an error: {error}"))?;
    let repo = response
        .json::<RepositorySchema>()
        .await
        .map_err(|error| format!("Repository JSON is invalid: {error}"))?;
    validate_repository_schema(&repo, allow_dev_http)?;
    Ok(repo)
}

pub(super) fn load_repository_schema_from_file(
    path: &str,
) -> Result<(RepositorySchema, String), String> {
    let path = normalize_repository_file_path(path)?;
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read repository JSON file: {error}"))?;
    let repo = serde_json::from_str::<RepositorySchema>(&raw)
        .map_err(|error| format!("Repository JSON is invalid: {error}"))?;
    validate_repository_schema(&repo, cfg!(debug_assertions))?;
    Ok((repo, file_url_for_path(&path)?))
}

pub(super) fn load_repository_schema_from_file_url(
    url: &str,
) -> Result<(RepositorySchema, String), String> {
    let parsed =
        Url::parse(url).map_err(|error| format!("Invalid file repository URL: {error}"))?;
    if parsed.scheme() != "file" {
        return Err(format!(
            "Unsupported repository URL scheme: {}",
            parsed.scheme()
        ));
    }
    let path = parsed
        .to_file_path()
        .map_err(|_| "Invalid file repository URL path.".to_string())?;
    load_repository_schema_from_file(&path.to_string_lossy())
}

pub(super) fn normalize_repository_file_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Repository file path cannot be empty.".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.exists() {
        return Err(format!(
            "Repository file does not exist: {}",
            path.display()
        ));
    }
    if !path.is_file() {
        return Err(format!("Repository path is not a file: {}", path.display()));
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| !extension.eq_ignore_ascii_case("json"))
        .unwrap_or(true)
    {
        return Err("Repository file must be a .json file.".to_string());
    }
    fs::canonicalize(&path).map_err(|error| format!("Failed to inspect repository file: {error}"))
}

pub(super) fn file_url_for_path(path: &Path) -> Result<String, String> {
    Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| format!("Failed to convert path to file URL: {}", path.display()))
}

pub(super) fn is_file_repository_url(url: &str) -> bool {
    Url::parse(url)
        .map(|parsed| parsed.scheme() == "file")
        .unwrap_or(false)
}

pub(super) fn load_builtin_demo_repository() -> Result<RepositorySchema, String> {
    let repo = builtin_demo::repository_schema()?;
    builtin_demo::verify_embedded_assets(&repo)?;
    Ok(repo)
}

pub(crate) fn repair_library_state(
    store: &mut RepositoryStore,
    data_dir: &Path,
) -> Result<RepairLibraryReport, String> {
    const LEGACY_REPOSITORY_ID: &str = "retrohydra-demo";
    const LEGACY_DEMO_URL: &str = "http://localhost:3000/demo-repository.json";
    const LEGACY_DEMO_GAME_IDS: &[&str] = &[
        "retrohydra-demo::nes_http_smoke",
        "retrohydra-demo::ps1_magnet_transport_smoke",
    ];
    const LEGACY_DEMO_ASSET_IDS: &[&str] = &[
        "retrohydra-demo::ps1_bios_scph1001",
        "retrohydra-demo::switch_prod_keys",
    ];

    let Some(url) = store.get_repository_url(LEGACY_REPOSITORY_ID)? else {
        return Ok(RepairLibraryReport {
            repaired: false,
            repository_id: None,
            removed_paths: Vec::new(),
        });
    };
    if url.trim() != LEGACY_DEMO_URL {
        return Ok(RepairLibraryReport {
            repaired: false,
            repository_id: Some(LEGACY_REPOSITORY_ID.to_string()),
            removed_paths: Vec::new(),
        });
    }

    let mut stale_paths = Vec::new();
    for game_id in LEGACY_DEMO_GAME_IDS {
        if let Some(path) = store
            .get_download(game_id)?
            .and_then(|record| record.local_path)
        {
            stale_paths.push(path);
        }
        if let Some(path) = store
            .get_torrent_download(game_id)?
            .map(|record| record.save_dir)
        {
            stale_paths.push(path);
        }
    }

    let repo = load_builtin_demo_repository()?;
    store.store_repository(builtin_demo::BUILTIN_DEMO_REPOSITORY_URL, &repo)?;
    for game_id in LEGACY_DEMO_GAME_IDS {
        store.delete_game_download_state(game_id)?;
    }
    for asset_id in LEGACY_DEMO_ASSET_IDS {
        store.delete_asset_state(asset_id)?;
    }
    store.disconnect_repository(LEGACY_REPOSITORY_ID)?;

    let download_root = store
        .get_config("download_root")?
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("Games"));
    let mut removed_paths = Vec::new();
    for stale_path in stale_paths {
        let path = PathBuf::from(&stale_path);
        if path.exists() {
            remove_path_if_allowed(data_dir, &download_root, &path)?;
            removed_paths.push(stale_path);
        }
    }

    Ok(RepairLibraryReport {
        repaired: true,
        repository_id: Some(LEGACY_REPOSITORY_ID.to_string()),
        removed_paths,
    })
}

pub(super) fn build_repository_preview(url: &str, repo: &RepositorySchema) -> RepositoryPreview {
    let raw_json = serde_json::to_vec(repo).unwrap_or_default();
    let content_hash = repo
        .metadata
        .content_hash
        .clone()
        .unwrap_or_else(|| hex::encode(Sha256::digest(&raw_json)));
    RepositoryPreview {
        url: url.to_string(),
        id: repo.metadata.id.clone(),
        name: repo.metadata.name.clone(),
        version: repo.metadata.version.clone(),
        maintainer: repo.metadata.maintainer.clone(),
        homepage_url: repo.metadata.homepage_url.clone(),
        license: repo.metadata.license.clone(),
        trust_level: repo
            .metadata
            .trust_level
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        catalog_count: repo.catalog.len(),
        system_file_count: repo.system_files.len(),
        has_executable_assets: repo.system_files.iter().any(|asset| asset.executable),
        content_hash,
    }
}
