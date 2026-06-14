use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::game_files::{self, GamePathErrorKind};
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

    fn game_file_corrupt(game_id: &str, path: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            kind: "GameFileCorrupt".to_string(),
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
#[allow(clippy::result_large_err)]
pub fn launch_game(
    game_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<LaunchReport, LaunchFailure> {
    let (emulator_path, game_path, launch_args_template, expected_extensions, preferred_file) = {
        let store = state
            .store
            .lock()
            .map_err(|_| LaunchFailure::spawn_failed("Repository store lock is poisoned."))?;
        let game = store
            .get_game(&game_id)
            .map_err(LaunchFailure::spawn_failed)?
            .ok_or_else(|| LaunchFailure::spawn_failed(format!("Unknown game: {game_id}")))?;

        let profile = crate::commands::resolve_known_profile(&game);
        let setup_state = crate::commands::build_game_setup_state(&store, &state.data_dir, &game)
            .map_err(LaunchFailure::spawn_failed)?;
        if let Some(profile_id) = setup_state.unsupported_profile_id.as_deref() {
            return Err(LaunchFailure::spawn_failed(format!(
                "Unsupported setup profile: {profile_id}"
            )));
        }

        let (emulator_config_path, emulator_config_launch_args) =
            if let Some(profile) = profile.as_ref() {
                let config = store
                    .get_profile_emulator_config(&profile.id)
                    .map_err(LaunchFailure::spawn_failed)?;
                (
                    config.as_ref().and_then(|config| config.exe_path.clone()),
                    config.and_then(|config| config.launch_args_template),
                )
            } else {
                let config = store
                    .get_emulator_config(&game.platform)
                    .map_err(LaunchFailure::spawn_failed)?;
                (
                    config.as_ref().and_then(|config| config.exe_path.clone()),
                    config.and_then(|config| config.launch_args_template),
                )
            };
        let emulator_path = emulator_config_path
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

        let game_path = resolve_downloaded_game_path(&store, &game.id)?;

        let mut corrupt_assets = setup_state
            .system_files
            .iter()
            .filter(|item| item.required && item.status == "corrupt")
            .map(|item| item.label.clone())
            .collect::<Vec<_>>();
        corrupt_assets.extend(
            setup_state
                .repository_requirements
                .iter()
                .filter(|item| item.status == "corrupt")
                .map(|item| item.asset.display_name.clone()),
        );
        if !corrupt_assets.is_empty() {
            return Err(LaunchFailure::system_file_corrupt(&game.id, corrupt_assets));
        }

        let mut missing_assets = setup_state
            .system_files
            .iter()
            .filter(|item| item.required && item.status != "ready")
            .map(|item| item.label.clone())
            .collect::<Vec<_>>();
        missing_assets.extend(
            setup_state
                .repository_requirements
                .iter()
                .filter(|item| item.status != "ready" || !item.trusted)
                .map(|item| item.asset.display_name.clone()),
        );
        if !missing_assets.is_empty() {
            return Err(LaunchFailure::system_files_missing(
                &game.id,
                missing_assets,
            ));
        }

        let expected_extensions =
            crate::commands::resolved_expected_extensions(&game, profile.as_ref());
        let preferred_file = game
            .launch
            .as_ref()
            .and_then(|launch| launch.preferred_file.clone())
            .or_else(|| {
                profile
                    .as_ref()
                    .and_then(|profile| profile.launch.preferred_file.clone())
            });
        let launch_args_template = game
            .launch
            .as_ref()
            .and_then(|launch| launch.args_template.clone())
            .or_else(|| {
                profile
                    .as_ref()
                    .map(|profile| profile.launch.args_template.clone())
            })
            .or(emulator_config_launch_args)
            .filter(|template| !template.trim().is_empty())
            .unwrap_or_else(|| "{game_path}".to_string());

        (
            emulator_path,
            game_path,
            launch_args_template,
            expected_extensions,
            preferred_file,
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

    let expected_extensions = game_files::normalize_expected_extensions(&expected_extensions)
        .map_err(LaunchFailure::spawn_failed)?;
    let resolved_game_path = game_files::resolve_game_path(
        Path::new(&game_path),
        &expected_extensions,
        preferred_file.as_deref(),
    )
    .map_err(|error| match error.kind {
        GamePathErrorKind::Missing => {
            LaunchFailure::game_file_missing(&game_id, Some(&game_path), error.message)
        }
        GamePathErrorKind::Corrupt => {
            LaunchFailure::game_file_corrupt(&game_id, Some(&game_path), error.message)
        }
    })?;
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

#[allow(clippy::result_large_err)]
fn resolve_downloaded_game_path(
    store: &crate::storage::RepositoryStore,
    game_id: &str,
) -> Result<String, LaunchFailure> {
    if let Some(local_path) = store
        .get_download(game_id)
        .map_err(LaunchFailure::spawn_failed)?
        .filter(|record| matches!(record.status.as_str(), "ready" | "completed"))
        .and_then(|record| record.local_path)
    {
        return Ok(local_path);
    }

    if let Some(save_dir) = store
        .get_torrent_download(game_id)
        .map_err(LaunchFailure::spawn_failed)?
        .filter(|record| record.status == "completed")
        .map(|record| record.save_dir)
    {
        return Ok(save_dir);
    }

    Err(LaunchFailure::game_file_missing(
        game_id,
        None,
        "Game is not downloaded yet.",
    ))
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
    use super::{parse_launch_args, resolve_downloaded_game_path, validate_emulator_path};
    use crate::storage::RepositoryStore;

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
    fn launch_args_receive_resolved_game_file_path() {
        let args = parse_launch_args(
            "C:/Emulators/retro.exe",
            "C:/Games/Sonic/Sonic.nsp",
            "-f -g {game_path}",
        )
        .unwrap();

        assert_eq!(args, vec!["-f", "-g", "C:/Games/Sonic/Sonic.nsp"]);
    }

    #[test]
    fn completed_torrent_record_resolves_as_launch_path() {
        let temp = tempfile::tempdir().unwrap();
        let store = RepositoryStore::open(&temp.path().join("fusion-launcher.db")).unwrap();
        let game_dir = temp.path().join("downloaded-game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let game_dir_string = game_dir.to_string_lossy().to_string();
        store
            .upsert_torrent_download_start(
                "repo::game",
                "magnet:?xt=urn:btih:abc",
                &game_dir_string,
            )
            .unwrap();
        store
            .update_torrent_progress("repo::game", "completed", 100.0, 100, 100, 0, 0, 0)
            .unwrap();

        let resolved = resolve_downloaded_game_path(&store, "repo::game").unwrap();

        assert_eq!(resolved, game_dir_string);
    }
}
