use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::build_requirements_report;
use crate::AppState;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x00000008;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchReport {
    pub pid: u32,
    pub executable: String,
    pub game_path: String,
    pub resolved_game_path: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchFailure {
    pub kind: String,
    pub platform: Option<String>,
    pub game_id: Option<String>,
    pub path: Option<String>,
    pub assets: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameExitedEvent {
    game_id: String,
    pid: u32,
    playtime_secs: u64,
}

impl LaunchFailure {
    fn emulator_not_configured(platform: &str) -> Self {
        Self {
            kind: "EmulatorNotConfigured".to_string(),
            platform: Some(platform.to_string()),
            game_id: None,
            path: None,
            assets: Vec::new(),
            message: Some("Configure an emulator for this platform before launching.".to_string()),
        }
    }

    fn emulator_file_missing(platform: &str, path: &str) -> Self {
        Self {
            kind: "EmulatorFileMissing".to_string(),
            platform: if platform.is_empty() {
                None
            } else {
                Some(platform.to_string())
            },
            game_id: None,
            path: Some(path.to_string()),
            assets: Vec::new(),
            message: Some("The configured emulator executable no longer exists.".to_string()),
        }
    }

    fn game_file_missing(game_id: &str, path: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            kind: "GameFileMissing".to_string(),
            platform: None,
            game_id: Some(game_id.to_string()),
            path: path.map(ToString::to_string),
            assets: Vec::new(),
            message: Some(message.into()),
        }
    }

    fn system_files_missing(game_id: &str, assets: Vec<String>) -> Self {
        Self {
            kind: "SystemFilesMissing".to_string(),
            platform: None,
            game_id: Some(game_id.to_string()),
            path: None,
            assets,
            message: Some("Required system files are missing.".to_string()),
        }
    }

    fn system_file_corrupt(game_id: &str, assets: Vec<String>) -> Self {
        Self {
            kind: "SystemFileCorrupt".to_string(),
            platform: None,
            game_id: Some(game_id.to_string()),
            path: None,
            assets,
            message: Some("One or more required system files failed verification.".to_string()),
        }
    }

    fn already_running(game_id: &str) -> Self {
        Self {
            kind: "AlreadyRunning".to_string(),
            platform: None,
            game_id: Some(game_id.to_string()),
            path: None,
            assets: Vec::new(),
            message: Some("This game is already running.".to_string()),
        }
    }

    fn spawn_failed(message: impl Into<String>) -> Self {
        Self {
            kind: "SpawnFailed".to_string(),
            platform: None,
            game_id: None,
            path: None,
            assets: Vec::new(),
            message: Some(message.into()),
        }
    }
}

#[tauri::command]
pub fn launch_game(
    game_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<LaunchReport, LaunchFailure> {
    let (emulator_path, game_path, launch_args_template, expected_extensions) = {
        let store = state
            .store
            .lock()
            .map_err(|_| LaunchFailure::spawn_failed("Repository store lock is poisoned."))?;
        let game = store
            .get_game(&game_id)
            .map_err(LaunchFailure::spawn_failed)?
            .ok_or_else(|| LaunchFailure::spawn_failed(format!("Unknown game: {game_id}")))?;

        let config = store
            .get_emulator_config(&game.platform)
            .map_err(LaunchFailure::spawn_failed)?
            .ok_or_else(|| LaunchFailure::emulator_not_configured(&game.platform))?;
        let emulator_path = config
            .exe_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| LaunchFailure::emulator_not_configured(&game.platform))?
            .to_string();

        if !Path::new(&emulator_path).exists() {
            return Err(LaunchFailure::emulator_file_missing(
                &game.platform,
                &emulator_path,
            ));
        }
        if let Err(message) = validate_emulator_path(Path::new(&emulator_path)) {
            return Err(LaunchFailure::spawn_failed(message));
        }

        let download = store
            .get_download(&game.id)
            .map_err(LaunchFailure::spawn_failed)?
            .filter(|record| record.status == "ready")
            .and_then(|record| record.local_path);
        let game_path = download.ok_or_else(|| {
            LaunchFailure::game_file_missing(&game.id, None, "Game is not downloaded yet.")
        })?;

        let requirements = build_requirements_report(&store, &state.data_dir, &game)
            .map_err(LaunchFailure::spawn_failed)?;
        let corrupt_assets = requirements
            .requirements
            .iter()
            .filter(|item| item.status == "corrupt")
            .map(|item| item.asset.display_name.clone())
            .collect::<Vec<_>>();
        if !corrupt_assets.is_empty() {
            return Err(LaunchFailure::system_file_corrupt(&game.id, corrupt_assets));
        }

        let missing_assets = requirements
            .requirements
            .iter()
            .filter(|item| item.status != "ready" || !item.trusted)
            .map(|item| item.asset.display_name.clone())
            .collect::<Vec<_>>();
        if !missing_assets.is_empty() {
            return Err(LaunchFailure::system_files_missing(
                &game.id,
                missing_assets,
            ));
        }

        (
            emulator_path,
            game_path,
            config
                .launch_args_template
                .filter(|template| !template.trim().is_empty())
                .unwrap_or_else(|| "{game_path}".to_string()),
            game.expected_extensions,
        )
    };

    {
        let running = state
            .running_games
            .lock()
            .map_err(|_| LaunchFailure::spawn_failed("Running game state lock is poisoned."))?;
        if running.contains_key(&game_id) {
            return Err(LaunchFailure::already_running(&game_id));
        }
    }

    let expected_extensions =
        normalize_expected_extensions(&expected_extensions).map_err(LaunchFailure::spawn_failed)?;
    let resolved_game_path = resolve_game_path(Path::new(&game_path), &expected_extensions)
        .map_err(|message| LaunchFailure::game_file_missing(&game_id, Some(&game_path), message))?;
    let resolved_game_path_string = resolved_game_path.to_string_lossy().to_string();
    let args = parse_launch_args(
        &emulator_path,
        &resolved_game_path_string,
        &launch_args_template,
    )
    .map_err(LaunchFailure::spawn_failed)?;

    let mut command = Command::new(&emulator_path);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(parent) = Path::new(&emulator_path).parent() {
        command.current_dir(parent);
    }

    #[cfg(windows)]
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);

    let child = command.spawn().map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            LaunchFailure::emulator_file_missing("", &emulator_path)
        } else {
            LaunchFailure::spawn_failed(launch_error(&emulator_path, error))
        }
    })?;
    let pid = child.id();

    {
        let mut running = state
            .running_games
            .lock()
            .map_err(|_| LaunchFailure::spawn_failed("Running game state lock is poisoned."))?;
        running.insert(game_id.clone(), pid);
    }

    let running_games = Arc::clone(&state.running_games);
    let app_handle = app.clone();
    let thread_game_id = game_id.clone();
    thread::spawn(move || {
        let started_at = Instant::now();
        let mut child = child;
        let _ = child.wait();
        if let Ok(mut running) = running_games.lock() {
            running.remove(&thread_game_id);
        }
        let _ = app_handle.emit(
            "game:exited",
            GameExitedEvent {
                game_id: thread_game_id,
                pid,
                playtime_secs: started_at.elapsed().as_secs(),
            },
        );
    });

    Ok(LaunchReport {
        pid,
        executable: emulator_path,
        game_path,
        resolved_game_path: resolved_game_path_string,
        args,
    })
}

fn validate_emulator_path(emulator_path: &Path) -> Result<(), String> {
    if !emulator_path.exists() {
        return Err(format!(
            "Emulator executable not found: {}",
            emulator_path.display()
        ));
    }
    if !emulator_path.is_file() {
        return Err(format!(
            "Emulator path is not a file: {}",
            emulator_path.display()
        ));
    }

    Ok(())
}

fn resolve_game_path(game_path: &Path, expected_extensions: &[String]) -> Result<PathBuf, String> {
    if !game_path.exists() {
        return Err(format!("Game path not found: {}", game_path.display()));
    }

    if game_path.is_file() {
        validate_game_file(game_path, expected_extensions)?;
        return make_absolute(game_path);
    }

    if !game_path.is_dir() {
        return Err(format!(
            "Game path is not a file or directory: {}",
            game_path.display()
        ));
    }

    scan_game_directory(game_path, expected_extensions)?
        .map(|candidate| candidate.path)
        .ok_or_else(|| {
            format!(
                "No game file found in {} matching extensions: {}",
                game_path.display(),
                expected_extensions.join(", ")
            )
        })
}

fn parse_launch_args(
    emulator_path: &str,
    game_path: &str,
    launch_args_template: &str,
) -> Result<Vec<String>, String> {
    let template = if launch_args_template.trim().is_empty() {
        "{game_path}"
    } else {
        launch_args_template
    };

    let parsed = shlex::split(template)
        .ok_or_else(|| "Invalid launch arguments template: malformed quotes".to_string())?;

    parsed
        .iter()
        .map(|arg| expand_placeholders(arg, emulator_path, game_path))
        .collect()
}

fn normalize_expected_extensions(expected_extensions: &[String]) -> Result<Vec<String>, String> {
    if expected_extensions.is_empty() {
        return Err("Expected extensions cannot be empty.".to_string());
    }

    let mut normalized = Vec::new();
    for extension in expected_extensions {
        let extension = extension.trim().to_lowercase();
        if extension.len() <= 1
            || !extension.starts_with('.')
            || !extension
                .chars()
                .skip(1)
                .all(|char| char.is_ascii_alphanumeric())
        {
            return Err(format!(
                "Invalid expected extension: {extension}. Extensions must look like .nsp"
            ));
        }
        if !normalized.contains(&extension) {
            normalized.push(extension);
        }
    }

    Ok(normalized)
}

fn validate_game_file(game_path: &Path, expected_extensions: &[String]) -> Result<(), String> {
    let metadata = game_path
        .metadata()
        .map_err(|error| format!("Failed to inspect game path: {error}"))?;
    if metadata.len() == 0 {
        return Err(format!("Game file is empty: {}", game_path.display()));
    }

    if !file_matches_extensions(game_path, expected_extensions) {
        return Err(format!(
            "Game file extension is not allowed: {}. Expected: {}",
            game_path.display(),
            expected_extensions.join(", ")
        ));
    }

    Ok(())
}

#[derive(Debug)]
struct GameFileCandidate {
    path: PathBuf,
    size: u64,
}

fn scan_game_directory(
    game_dir: &Path,
    expected_extensions: &[String],
) -> Result<Option<GameFileCandidate>, String> {
    let mut best: Option<GameFileCandidate> = None;

    scan_game_directory_inner(game_dir, expected_extensions, &mut best)?;

    Ok(best)
}

fn scan_game_directory_inner(
    directory: &Path,
    expected_extensions: &[String],
    best: &mut Option<GameFileCandidate>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        format!(
            "Failed to read game directory {}: {error}",
            directory.display()
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read game directory entry in {}: {error}",
                directory.display()
            )
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect game file {}: {error}", path.display()))?;

        if file_type.is_dir() {
            scan_game_directory_inner(&path, expected_extensions, best)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let metadata = entry
            .metadata()
            .map_err(|error| format!("Failed to inspect game file {}: {error}", path.display()))?;
        if metadata.len() == 0 {
            continue;
        }

        if file_matches_extensions(&path, expected_extensions) {
            let path = make_absolute(&path)?;
            let candidate = GameFileCandidate {
                path,
                size: metadata.len(),
            };

            if best
                .as_ref()
                .map(|current| candidate.size > current.size)
                .unwrap_or(true)
            {
                *best = Some(candidate);
            }
        }
    }

    Ok(())
}

fn file_matches_extensions(path: &Path, expected_extensions: &[String]) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    let extension = format!(".{}", extension.to_lowercase());

    expected_extensions
        .iter()
        .any(|expected_extension| expected_extension == &extension)
}

fn make_absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current_dir| current_dir.join(path))
        .map_err(|error| format!("Failed to resolve absolute game path: {error}"))
}

fn expand_placeholders(arg: &str, emulator_path: &str, game_path: &str) -> Result<String, String> {
    let mut expanded = String::with_capacity(arg.len());
    let mut remaining = arg;

    loop {
        match (remaining.find('{'), remaining.find('}')) {
            (None, None) => {
                expanded.push_str(remaining);
                return Ok(expanded);
            }
            (None, Some(_)) => {
                return Err("Invalid launch arguments template: unmatched '}'".to_string());
            }
            (Some(open), Some(close)) if close < open => {
                return Err("Invalid launch arguments template: unmatched '}'".to_string());
            }
            (Some(open), _) => {
                expanded.push_str(&remaining[..open]);

                let after_open = &remaining[open + 1..];
                let close = after_open.find('}').ok_or_else(|| {
                    "Invalid launch arguments template: unmatched '{'".to_string()
                })?;
                let placeholder = &after_open[..close];
                let replacement = match placeholder {
                    "game_path" => game_path,
                    "emulator_path" => emulator_path,
                    _ => {
                        return Err(format!(
                            "Invalid launch arguments template: unknown placeholder {{{placeholder}}}"
                        ));
                    }
                };

                expanded.push_str(replacement);
                remaining = &after_open[close + 1..];
            }
        }
    }
}

fn launch_error(emulator_path: &str, error: std::io::Error) -> String {
    if error.kind() == ErrorKind::NotFound {
        format!("Emulator executable not found: {emulator_path}")
    } else {
        format!("Failed to launch emulator: {error}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_expected_extensions, parse_launch_args, resolve_game_path, validate_emulator_path,
    };

    #[test]
    fn parses_basic_game_path_template() {
        let args = parse_launch_args(
            "C:/Emulators/retro.exe",
            "C:/Games/Sonic.bin",
            "-f -g {game_path}",
        )
        .unwrap();

        assert_eq!(args, vec!["-f", "-g", "C:/Games/Sonic.bin"]);
    }

    #[test]
    fn preserves_quoted_game_path_as_one_arg() {
        let args = parse_launch_args(
            "C:/Emulators/retro.exe",
            "C:/Games/Sonic The Hedgehog.bin",
            "--rom=\"{game_path}\"",
        )
        .unwrap();

        assert_eq!(args, vec!["--rom=C:/Games/Sonic The Hedgehog.bin"]);
    }

    #[test]
    fn blank_template_defaults_to_game_path() {
        let args = parse_launch_args("emu", "C:/Games/Sonic.bin", "   ").unwrap();

        assert_eq!(args, vec!["C:/Games/Sonic.bin"]);
    }

    #[test]
    fn malformed_quotes_return_error() {
        let error = parse_launch_args("emu", "game", "\"{game_path}").unwrap_err();

        assert_eq!(error, "Invalid launch arguments template: malformed quotes");
    }

    #[test]
    fn unknown_placeholders_return_error() {
        let error = parse_launch_args("emu", "game", "{rom_path}").unwrap_err();

        assert_eq!(
            error,
            "Invalid launch arguments template: unknown placeholder {rom_path}"
        );
    }

    #[test]
    fn missing_emulator_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let emulator_path = temp.path().join("missing-emulator.exe");

        let error = validate_emulator_path(&emulator_path).unwrap_err();

        assert_eq!(
            error,
            format!("Emulator executable not found: {}", emulator_path.display())
        );
    }

    #[test]
    fn missing_game_path_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("missing-game.bin");

        let expected_extensions = normalize_expected_extensions(&[".bin".to_string()]).unwrap();
        let error = resolve_game_path(&game_path, &expected_extensions).unwrap_err();

        assert_eq!(
            error,
            format!("Game path not found: {}", game_path.display())
        );
    }

    #[test]
    fn game_directory_picks_largest_matching_file() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("game-directory");
        std::fs::create_dir(&game_path).unwrap();
        std::fs::write(game_path.join("tiny.nsp"), b"tiny").unwrap();
        std::fs::write(game_path.join("larger.xci"), b"larger file").unwrap();
        std::fs::write(game_path.join("ignored.txt"), b"not a game").unwrap();

        let expected_extensions =
            normalize_expected_extensions(&[".nsp".to_string(), ".xci".to_string()]).unwrap();
        let resolved = resolve_game_path(&game_path, &expected_extensions).unwrap();

        assert_eq!(resolved, game_path.join("larger.xci"));
    }

    #[test]
    fn game_directory_scan_is_case_insensitive() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("game-directory");
        std::fs::create_dir(&game_path).unwrap();
        std::fs::write(game_path.join("GAME.NSP"), b"game").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
        let resolved = resolve_game_path(&game_path, &expected_extensions).unwrap();

        assert_eq!(resolved, game_path.join("GAME.NSP"));
    }

    #[test]
    fn empty_game_file_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("empty-game.nsp");
        std::fs::write(&game_path, b"").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
        let error = resolve_game_path(&game_path, &expected_extensions).unwrap_err();

        assert_eq!(
            error,
            format!("Game file is empty: {}", game_path.display())
        );
    }

    #[test]
    fn no_matching_game_file_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("game-directory");
        std::fs::create_dir(&game_path).unwrap();
        std::fs::write(game_path.join("readme.txt"), b"not a game").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
        let error = resolve_game_path(&game_path, &expected_extensions).unwrap_err();

        assert_eq!(
            error,
            format!(
                "No game file found in {} matching extensions: .nsp",
                game_path.display()
            )
        );
    }

    #[test]
    fn disallowed_game_file_extension_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("game.txt");
        std::fs::write(&game_path, b"game").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
        let error = resolve_game_path(&game_path, &expected_extensions).unwrap_err();

        assert_eq!(
            error,
            format!(
                "Game file extension is not allowed: {}. Expected: .nsp",
                game_path.display()
            )
        );
    }

    #[test]
    fn launch_args_receive_resolved_game_file_path() {
        let args = parse_launch_args(
            "C:/Emulators/retro.exe",
            "C:/Games/Sonic/Sonic.nsp",
            "-f -g {game_path}",
        )
        .unwrap();

        assert_eq!(args, vec!["-f", "-g", "C:/Games/Sonic/Sonic.nsp"]);
    }
}
