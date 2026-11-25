use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GamePathErrorKind {
    Missing,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamePathError {
    pub kind: GamePathErrorKind,
    pub message: String,
}

#[derive(Debug)]
struct GameFileCandidate {
    path: PathBuf,
    size: u64,
}

pub fn normalize_expected_extensions(
    expected_extensions: &[String],
) -> Result<Vec<String>, String> {
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

pub fn resolve_game_path(
    game_path: &Path,
    expected_extensions: &[String],
    preferred_file: Option<&str>,
) -> Result<PathBuf, GamePathError> {
    if !game_path.exists() {
        return Err(missing(format!(
            "Game path not found: {}",
            game_path.display()
        )));
    }

    if game_path.is_file() {
        validate_game_file(game_path, expected_extensions)?;
        return make_absolute(game_path);
    }

    if !game_path.is_dir() {
        return Err(corrupt(format!(
            "Game path is not a file or directory: {}",
            game_path.display()
        )));
    }

    if let Some(preferred_file) = preferred_file.and_then(safe_relative_path) {
        let candidate = game_path.join(preferred_file);
        if !candidate.exists() {
            return Err(missing(format!(
                "Preferred game file not found: {}",
                candidate.display()
            )));
        }
        let canonical_game_dir = game_path
            .canonicalize()
            .map_err(|e| corrupt(format!("Failed to canonicalize game directory: {e}")))?;
        let canonical_candidate = candidate
            .canonicalize()
            .map_err(|e| corrupt(format!("Failed to canonicalize game candidate: {e}")))?;
        if !canonical_candidate.starts_with(&canonical_game_dir) {
            return Err(corrupt(format!(
                "Preferred game file escaped game directory: {}",
                candidate.display()
            )));
        }
        validate_game_file(&canonical_candidate, expected_extensions)?;
        return make_absolute(&candidate);
    }

    scan_game_directory(game_path, expected_extensions)?
        .map(|candidate| candidate.path)
        .ok_or_else(|| {
            missing(format!(
                "No game file found in {} matching extensions: {}",
                game_path.display(),
                expected_extensions.join(", ")
            ))
        })
}

pub fn inspect_game_path(
    game_path: &Path,
    expected_extensions: &[String],
    preferred_file: Option<&str>,
) -> (String, Option<String>) {
    if expected_extensions.is_empty() {
        return (
            "error".to_string(),
            Some("No expected game file extensions are configured.".to_string()),
        );
    }

    match normalize_expected_extensions(expected_extensions)
        .map_err(|message| GamePathError {
            kind: GamePathErrorKind::Corrupt,
            message,
        })
        .and_then(|extensions| resolve_game_path(game_path, &extensions, preferred_file))
    {
        Ok(resolved) => ("ready".to_string(), Some(resolved.display().to_string())),
        Err(error) => {
            let status = match error.kind {
                GamePathErrorKind::Missing => "missing",
                GamePathErrorKind::Corrupt => "corrupt",
            };
            (status.to_string(), Some(error.message))
        }
    }
}

fn validate_game_file(
    game_path: &Path,
    expected_extensions: &[String],
) -> Result<(), GamePathError> {
    let metadata = game_path
        .metadata()
        .map_err(|error| corrupt(format!("Failed to inspect game path: {error}")))?;
    if metadata.len() == 0 {
        return Err(corrupt(format!(
            "Game file is empty: {}",
            game_path.display()
        )));
    }

    if !file_matches_extensions(game_path, expected_extensions) {
        return Err(corrupt(format!(
            "Game file extension is not allowed: {}. Expected: {}",
            game_path.display(),
            expected_extensions.join(", ")
        )));
    }

    if file_extension(game_path).as_deref() == Some(".nes") {
        validate_ines_header(game_path)?;
    }

    Ok(())
}

fn scan_game_directory(
    game_dir: &Path,
    expected_extensions: &[String],
) -> Result<Option<GameFileCandidate>, GamePathError> {
    let mut best: Option<GameFileCandidate> = None;

    scan_game_directory_inner(game_dir, expected_extensions, &mut best)?;

    Ok(best)
}

fn scan_game_directory_inner(
    directory: &Path,
    expected_extensions: &[String],
    best: &mut Option<GameFileCandidate>,
) -> Result<(), GamePathError> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        missing(format!(
            "Failed to read game directory {}: {error}",
            directory.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            missing(format!(
                "Failed to read game directory entry in {}: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            corrupt(format!(
                "Failed to inspect game file {}: {error}",
                path.display()
            ))
        })?;

        if file_type.is_dir() {
            scan_game_directory_inner(&path, expected_extensions, best)?;
            continue;
        }

        if !file_type.is_file() || !file_matches_extensions(&path, expected_extensions) {
            continue;
        }

        validate_game_file(&path, expected_extensions)?;

        let metadata = entry.metadata().map_err(|error| {
            corrupt(format!(
                "Failed to inspect game file {}: {error}",
                path.display()
            ))
        })?;
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

    Ok(())
}

fn validate_ines_header(game_path: &Path) -> Result<(), GamePathError> {
    let mut file = File::open(game_path).map_err(|error| {
        corrupt(format!(
            "Failed to open NES game file {}: {error}",
            game_path.display()
        ))
    })?;
    let mut header = [0_u8; 4];
    file.read_exact(&mut header).map_err(|_| {
        corrupt(format!(
            "NES game file is too small to contain an iNES header: {}",
            game_path.display()
        ))
    })?;

    if header == *b"NES\x1A" {
        Ok(())
    } else {
        Err(corrupt(format!(
            "NES game file does not contain a valid iNES header: {}",
            game_path.display()
        )))
    }
}

fn file_matches_extensions(path: &Path, expected_extensions: &[String]) -> bool {
    let Some(extension) = file_extension(path) else {
        return false;
    };

    expected_extensions
        .iter()
        .any(|expected_extension| expected_extension == &extension)
}

fn file_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{}", extension.to_lowercase()))
}

fn safe_relative_path(input: &str) -> Option<PathBuf> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return None;
    }

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => safe.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}

fn make_absolute(path: &Path) -> Result<PathBuf, GamePathError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current_dir| current_dir.join(path))
        .map_err(|error| corrupt(format!("Failed to resolve absolute game path: {error}")))
}

fn missing(message: impl Into<String>) -> GamePathError {
    GamePathError {
        kind: GamePathErrorKind::Missing,
        message: message.into(),
    }
}

fn corrupt(message: impl Into<String>) -> GamePathError {
    GamePathError {
        kind: GamePathErrorKind::Corrupt,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_expected_extensions, resolve_game_path, safe_relative_path, GamePathErrorKind,
    };

    fn valid_nes_bytes() -> Vec<u8> {
        let mut bytes = b"NES\x1A".to_vec();
        bytes.extend([0_u8; 32]);
        bytes
    }

    #[test]
    fn missing_game_path_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("missing-game.bin");

        let expected_extensions = normalize_expected_extensions(&[".bin".to_string()]).unwrap();
        let error = resolve_game_path(&game_path, &expected_extensions, None).unwrap_err();

        assert_eq!(error.kind, GamePathErrorKind::Missing);
        assert_eq!(
            error.message,
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
        let resolved = resolve_game_path(&game_path, &expected_extensions, None).unwrap();

        assert_eq!(resolved, game_path.join("larger.xci"));
    }

    #[test]
    fn game_directory_scan_is_case_insensitive() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("game-directory");
        std::fs::create_dir(&game_path).unwrap();
        std::fs::write(game_path.join("GAME.NSP"), b"game").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
        let resolved = resolve_game_path(&game_path, &expected_extensions, None).unwrap();

        assert_eq!(resolved, game_path.join("GAME.NSP"));
    }

    #[test]
    fn preferred_file_is_used_inside_directory() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("game-directory");
        std::fs::create_dir(&game_path).unwrap();
        std::fs::write(game_path.join("wrong.nes"), valid_nes_bytes()).unwrap();
        std::fs::create_dir(game_path.join("roms")).unwrap();
        std::fs::write(game_path.join("roms").join("right.nes"), valid_nes_bytes()).unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nes".to_string()]).unwrap();
        let resolved =
            resolve_game_path(&game_path, &expected_extensions, Some("roms/right.nes")).unwrap();

        assert_eq!(resolved, game_path.join("roms").join("right.nes"));
    }

    #[test]
    fn empty_game_file_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("empty-game.nsp");
        std::fs::write(&game_path, b"").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
        let error = resolve_game_path(&game_path, &expected_extensions, None).unwrap_err();

        assert_eq!(error.kind, GamePathErrorKind::Corrupt);
        assert_eq!(
            error.message,
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
        let error = resolve_game_path(&game_path, &expected_extensions, None).unwrap_err();

        assert_eq!(error.kind, GamePathErrorKind::Missing);
        assert_eq!(
            error.message,
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
        let error = resolve_game_path(&game_path, &expected_extensions, None).unwrap_err();

        assert_eq!(error.kind, GamePathErrorKind::Corrupt);
        assert_eq!(
            error.message,
            format!(
                "Game file extension is not allowed: {}. Expected: .nsp",
                game_path.display()
            )
        );
    }

    #[test]
    fn invalid_nes_header_returns_corrupt() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("bad.nes");
        std::fs::write(&game_path, b"not an ines rom").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nes".to_string()]).unwrap();
        let error = resolve_game_path(&game_path, &expected_extensions, None).unwrap_err();

        assert_eq!(error.kind, GamePathErrorKind::Corrupt);
        assert!(error.message.contains("valid iNES header"));
    }

    #[test]
    fn valid_nes_header_is_accepted() {
        let temp = tempfile::tempdir().unwrap();
        let game_path = temp.path().join("good.nes");
        std::fs::write(&game_path, valid_nes_bytes()).unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nes".to_string()]).unwrap();
        let resolved = resolve_game_path(&game_path, &expected_extensions, None).unwrap();

        assert_eq!(resolved, game_path);
    }

    #[test]
    fn preferred_file_inside_dir_is_accepted() {
        let temp = tempfile::tempdir().unwrap();
        let game_dir = temp.path().join("game-dir");
        std::fs::create_dir(&game_dir).unwrap();
        let rom = game_dir.join("game.nsp");
        std::fs::write(&rom, b"rom-content").unwrap();

        let expected_extensions = normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
        let resolved =
            resolve_game_path(&game_dir, &expected_extensions, Some("game.nsp")).unwrap();

        assert_eq!(resolved, rom);
    }

    #[test]
    fn preferred_file_path_traversal_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let game_dir = temp.path().join("game-dir");
        std::fs::create_dir(&game_dir).unwrap();
        let rom = game_dir.join("game.nsp");
        std::fs::write(&rom, b"rom-content").unwrap();

        // Lexical traversal like ../other.nsp gets stripped by safe_relative_path -> falls back to scan or missing
        assert!(safe_relative_path("../outside.nsp").is_none());
        assert!(safe_relative_path("C:\\Windows\\System32\\cmd.exe").is_none());
        assert!(safe_relative_path("/etc/passwd").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn preferred_file_junction_escape_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let game_dir = temp.path().join("game-dir");
        let outside_dir = temp.path().join("outside-dir");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        let outside_rom = outside_dir.join("outside.nsp");
        std::fs::write(&outside_rom, b"outside-rom-content").unwrap();

        let junction = game_dir.join("escaped_junction");
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                junction.to_str().unwrap(),
                outside_dir.to_str().unwrap(),
            ])
            .status();

        if let Ok(s) = status {
            if s.success() {
                let expected_extensions =
                    normalize_expected_extensions(&[".nsp".to_string()]).unwrap();
                let result = resolve_game_path(
                    &game_dir,
                    &expected_extensions,
                    Some("escaped_junction/outside.nsp"),
                );
                assert!(result.is_err());
                let err = result.unwrap_err();
                assert_eq!(err.kind, GamePathErrorKind::Corrupt);
                assert!(err.message.contains("escaped game directory"));
            }
        }
    }
}
