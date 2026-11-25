//! Bounded, read-only storefront discovery. These paths are candidates only;
//! `LocalLocatorResolver` remains responsible for key hashes and volume identity.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use super::Cancellation;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use crate::fusion_source::resolver::StorefrontInventory;
use crate::fusion_source::schema::LocalStore;

const FILE_LIMIT: u64 = 1024 * 1024;
const ENTRY_LIMIT: usize = 4096;
const LIBRARY_LIMIT: usize = 128;
const DEPTH_LIMIT: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct StorefrontRoots {
    pub steam_roots: Vec<PathBuf>,
    pub gog_games: Vec<(Option<String>, PathBuf)>,
    pub epic_manifests: Option<PathBuf>,
}

/// Injectable registry boundary; implementations must use read-only access.
pub trait StorefrontRegistry: Send + Sync {
    fn steam_roots(&self, cancel: &Cancellation) -> Result<Vec<PathBuf>, FusionError>;
    fn gog_games(
        &self,
        cancel: &Cancellation,
    ) -> Result<Vec<(Option<String>, PathBuf)>, FusionError>;
}

struct WindowsRegistry;

#[cfg(windows)]
impl StorefrontRegistry for WindowsRegistry {
    fn steam_roots(&self, cancel: &Cancellation) -> Result<Vec<PathBuf>, FusionError> {
        use winreg::enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        };
        use winreg::RegKey;
        let mut paths = Vec::new();
        for (hive, key) in [
            (HKEY_CURRENT_USER, "Software\\Valve\\Steam"),
            (HKEY_LOCAL_MACHINE, "Software\\Valve\\Steam"),
            (HKEY_LOCAL_MACHINE, "Software\\WOW6432Node\\Valve\\Steam"),
        ] {
            for view in [KEY_WOW64_32KEY, KEY_WOW64_64KEY] {
                cancel.check()?;
                if let Ok(key) = RegKey::predef(hive).open_subkey_with_flags(key, KEY_READ | view) {
                    for field in ["SteamPath", "InstallPath"] {
                        if let Ok(value) = key.get_value::<String, _>(field) {
                            paths.push(PathBuf::from(value));
                        }
                    }
                }
            }
        }
        Ok(paths)
    }

    fn gog_games(
        &self,
        cancel: &Cancellation,
    ) -> Result<Vec<(Option<String>, PathBuf)>, FusionError> {
        use winreg::enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        };
        use winreg::RegKey;
        let mut games = Vec::new();
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_32KEY, KEY_WOW64_64KEY] {
                cancel.check()?;
                let Ok(root) = RegKey::predef(hive)
                    .open_subkey_with_flags("Software\\GOG.com\\Games", KEY_READ | view)
                else {
                    continue;
                };
                for name in root.enum_keys().take(ENTRY_LIMIT).flatten() {
                    cancel.check()?;
                    if games.len() == ENTRY_LIMIT {
                        return Ok(games);
                    }
                    let Ok(key) = root.open_subkey_with_flags(&name, KEY_READ | view) else {
                        continue;
                    };
                    if let Ok(value) = key.get_value::<String, _>("path") {
                        games.push((Some(name), PathBuf::from(value)));
                    }
                }
            }
        }
        Ok(games)
    }
}

#[cfg(not(windows))]
impl StorefrontRegistry for WindowsRegistry {
    fn steam_roots(&self, cancel: &Cancellation) -> Result<Vec<PathBuf>, FusionError> {
        cancel.check()?;
        Ok(Vec::new())
    }
    fn gog_games(
        &self,
        cancel: &Cancellation,
    ) -> Result<Vec<(Option<String>, PathBuf)>, FusionError> {
        cancel.check()?;
        Ok(Vec::new())
    }
}

pub struct WindowsStorefrontInventory {
    roots: StorefrontRoots,
    registry: Option<Arc<dyn StorefrontRegistry>>,
}

impl Default for WindowsStorefrontInventory {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsStorefrontInventory {
    pub fn new() -> Self {
        let mut roots = StorefrontRoots::default();
        for name in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(value) = std::env::var_os(name) {
                roots.steam_roots.push(PathBuf::from(value).join("Steam"));
            }
        }
        #[cfg(windows)]
        roots
            .steam_roots
            .push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        roots.epic_manifests = std::env::var_os("ProgramData")
            .map(|value| PathBuf::from(value).join("Epic/EpicGamesLauncher/Data/Manifests"));
        Self {
            roots,
            registry: Some(Arc::new(WindowsRegistry)),
        }
    }

    /// Explicit roots avoid reading the machine's registry, including in tests.
    pub fn with_roots(roots: StorefrontRoots) -> Self {
        Self {
            roots,
            registry: None,
        }
    }

    pub fn with_registry(mut self, registry: Arc<dyn StorefrontRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }
}

struct AbortScan(Cancellation);
impl Drop for AbortScan {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct ScanCancel {
    caller: Cancellation,
    abort: Cancellation,
}
impl ScanCancel {
    fn check(&self) -> Result<(), FusionError> {
        self.caller.check()?;
        self.abort.check()
    }
}

#[async_trait]
impl StorefrontInventory for WindowsStorefrontInventory {
    async fn candidates(
        &self,
        store: LocalStore,
        app_id: Option<&str>,
        cancel: &Cancellation,
    ) -> Result<Vec<PathBuf>, FusionError> {
        cancel.check()?;
        let mut roots = self.roots.clone();
        let registry = self.registry.clone();
        let app_id = app_id.map(str::to_owned);
        let abort = Cancellation::default();
        let _abort_guard = AbortScan(abort.clone());
        let cancellation = ScanCancel {
            caller: cancel.clone(),
            abort,
        };
        tokio::task::spawn_blocking(move || {
            cancellation.check()?;
            if let Some(registry) = registry {
                match store {
                    LocalStore::Steam => roots
                        .steam_roots
                        .splice(0..0, registry.steam_roots(&cancellation.caller)?)
                        .for_each(drop),
                    LocalStore::Gog => roots
                        .gog_games
                        .extend(registry.gog_games(&cancellation.caller)?),
                    _ => {}
                }
            }
            let mut found = CandidateSet::default();
            match store {
                LocalStore::Steam => scan_steam(
                    &roots.steam_roots,
                    app_id.as_deref(),
                    &cancellation,
                    &mut found,
                )?,
                LocalStore::Gog => {
                    for (id, path) in roots.gog_games.into_iter().take(ENTRY_LIMIT) {
                        cancellation.check()?;
                        if app_id
                            .as_ref()
                            .is_none_or(|wanted| id.as_ref() == Some(wanted))
                        {
                            found.add(path);
                        }
                    }
                }
                LocalStore::Epic => {
                    if let Some(root) = roots.epic_manifests {
                        scan_epic(&root, app_id.as_deref(), &cancellation, &mut found)?;
                    }
                }
                LocalStore::Manual => {}
            }
            cancellation.check()?;
            Ok(found.paths)
        })
        .await
        .map_err(|_| FusionError::new(ErrorCode::Io, "Storefront inventory worker failed."))?
    }
}

#[derive(Default)]
struct CandidateSet {
    paths: Vec<PathBuf>,
    seen: BTreeSet<String>,
}
impl CandidateSet {
    fn add(&mut self, path: PathBuf) {
        if self.paths.len() < ENTRY_LIMIT
            && safe_directory(&path)
            && self
                .seen
                .insert(path.to_string_lossy().to_ascii_lowercase())
        {
            self.paths.push(path);
        }
    }
}

fn safe_directory(path: &Path) -> bool {
    path.is_absolute() && fs_safety::guard(path).is_ok()
}

fn read_bounded(path: &Path, cancel: &ScanCancel) -> Result<String, FusionError> {
    cancel.check()?;
    let mut file = fs_safety::open_read(path)?;
    if file
        .metadata()
        .map_err(|error| fs_safety::io_error("Inspect storefront file", path, error))?
        .len()
        > FILE_LIMIT
    {
        return Err(invalid("Storefront file exceeds 1 MiB."));
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        cancel.check()?;
        let count = file
            .read(&mut chunk)
            .map_err(|error| fs_safety::io_error("Read storefront file", path, error))?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > FILE_LIMIT as usize {
            return Err(invalid("Storefront file exceeds 1 MiB."));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(bytes).map_err(|_| invalid("Storefront file is not UTF-8."))
}

fn scan_steam(
    roots: &[PathBuf],
    app_id: Option<&str>,
    cancel: &ScanCancel,
    found: &mut CandidateSet,
) -> Result<(), FusionError> {
    let mut libraries = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots.iter().take(LIBRARY_LIMIT) {
        cancel.check()?;
        if !safe_directory(root) {
            continue;
        }
        if libraries.len() < LIBRARY_LIMIT {
            libraries.push(root.clone());
        }
        let file = root.join("steamapps/libraryfolders.vdf");
        match read_bounded(&file, cancel).and_then(|text| parse_vdf(&text)) {
            Ok(document) => {
                if let Some(VdfValue::Object(entries)) = lookup(&document, "libraryfolders") {
                    for (key, value) in entries {
                        cancel.check()?;
                        if !key.bytes().all(|byte| byte.is_ascii_digit()) {
                            continue;
                        }
                        let path = match value {
                            VdfValue::Text(value) => Some(value.as_str()),
                            VdfValue::Object(value) => text_field(value, "path"),
                        };
                        if let Some(path) = path {
                            if libraries.len() < LIBRARY_LIMIT {
                                libraries.push(PathBuf::from(path));
                            }
                        }
                    }
                }
            }
            Err(error) if error.code == ErrorCode::Cancelled => return Err(error),
            Err(_) => {}
        }
    }
    let mut remaining = ENTRY_LIMIT;
    for library in libraries.into_iter().take(LIBRARY_LIMIT) {
        cancel.check()?;
        if !seen.insert(library.to_string_lossy().to_ascii_lowercase()) {
            continue;
        }
        let steamapps = library.join("steamapps");
        let Ok(_guard) = fs_safety::guard(&steamapps) else {
            continue;
        };
        let Ok(entries) = fs::read_dir(&steamapps) else {
            continue;
        };
        for entry in entries.take(ENTRY_LIMIT).flatten() {
            cancel.check()?;
            if remaining == 0 {
                return Ok(());
            }
            remaining -= 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            match read_bounded(&entry.path(), cancel).and_then(|text| parse_vdf(&text)) {
                Ok(document) => {
                    if let Some(VdfValue::Object(app)) = lookup(&document, "AppState") {
                        if app_id.is_some_and(|wanted| text_field(app, "appid") != Some(wanted)) {
                            continue;
                        }
                        if let Some(directory) = text_field(app, "installdir") {
                            if let Ok(relative) = fs_safety::validate_relative_path(directory) {
                                if relative.components().count() == 1 {
                                    found.add(steamapps.join("common").join(relative));
                                }
                            }
                        }
                    }
                }
                Err(error) if error.code == ErrorCode::Cancelled => return Err(error),
                Err(_) => {}
            }
        }
        let common = steamapps.join("common");
        let Ok(_common_guard) = fs_safety::guard(&common) else {
            continue;
        };
        if let Ok(entries) = fs::read_dir(common) {
            for entry in entries.take(ENTRY_LIMIT).flatten() {
                cancel.check()?;
                if remaining == 0 {
                    return Ok(());
                }
                remaining -= 1;
                found.add(entry.path());
            }
        }
    }
    Ok(())
}

fn scan_epic(
    root: &Path,
    app_id: Option<&str>,
    cancel: &ScanCancel,
    found: &mut CandidateSet,
) -> Result<(), FusionError> {
    let Ok(_guard) = fs_safety::guard(root) else {
        return Ok(());
    };
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(());
    };
    for entry in entries.take(ENTRY_LIMIT).flatten() {
        cancel.check()?;
        if !entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("item"))
        {
            continue;
        }
        let parsed = read_bounded(&entry.path(), cancel).and_then(|text| {
            serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|_| invalid("Malformed Epic inventory."))
        });
        match parsed {
            Ok(value) => {
                if app_id.is_some_and(|id| {
                    !["AppName", "CatalogItemId", "MainGameCatalogItemId"]
                        .iter()
                        .any(|field| value.get(field).and_then(|value| value.as_str()) == Some(id))
                }) {
                    continue;
                }
                if let Some(path) = value
                    .get("InstallLocation")
                    .and_then(|value| value.as_str())
                {
                    found.add(PathBuf::from(path));
                }
            }
            Err(error) if error.code == ErrorCode::Cancelled => return Err(error),
            Err(_) => {}
        }
    }
    Ok(())
}

#[derive(Debug)]
enum VdfValue {
    Text(String),
    Object(Vec<(String, VdfValue)>),
}
fn lookup<'a>(entries: &'a [(String, VdfValue)], key: &str) -> Option<&'a VdfValue> {
    entries
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}
fn text_field<'a>(entries: &'a [(String, VdfValue)], key: &str) -> Option<&'a str> {
    match lookup(entries, key) {
        Some(VdfValue::Text(value)) => Some(value),
        _ => None,
    }
}
fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

fn parse_vdf(text: &str) -> Result<Vec<(String, VdfValue)>, FusionError> {
    if text.len() > FILE_LIMIT as usize {
        return Err(invalid("VDF exceeds 1 MiB."));
    }
    struct Parser<'a> {
        text: &'a [u8],
        index: usize,
        entries: usize,
    }
    impl Parser<'_> {
        fn whitespace(&mut self) {
            loop {
                while self
                    .text
                    .get(self.index)
                    .is_some_and(u8::is_ascii_whitespace)
                {
                    self.index += 1;
                }
                if self.text.get(self.index..self.index + 2) != Some(b"//") {
                    break;
                }
                while self
                    .text
                    .get(self.index)
                    .is_some_and(|value| *value != b'\n')
                {
                    self.index += 1;
                }
            }
        }
        fn word(&mut self) -> Result<String, FusionError> {
            self.whitespace();
            let mut value = Vec::new();
            let quoted = self.text.get(self.index) == Some(&b'"');
            if quoted {
                self.index += 1;
                loop {
                    let byte = *self
                        .text
                        .get(self.index)
                        .ok_or_else(|| invalid("Unterminated VDF string."))?;
                    self.index += 1;
                    match byte {
                        b'"' => break,
                        b'\\' => {
                            let next = *self
                                .text
                                .get(self.index)
                                .ok_or_else(|| invalid("Unterminated VDF escape."))?;
                            self.index += 1;
                            match next {
                                b'"' | b'\\' => value.push(next),
                                b'n' => value.push(b'\n'),
                                b'r' => value.push(b'\r'),
                                b't' => value.push(b'\t'),
                                _ => {
                                    value.push(b'\\');
                                    value.push(next);
                                }
                            }
                        }
                        value_byte => value.push(value_byte),
                    }
                }
            } else {
                while let Some(byte) = self.text.get(self.index) {
                    if byte.is_ascii_whitespace() || matches!(*byte, b'{' | b'}' | b'"') {
                        break;
                    }
                    value.push(*byte);
                    self.index += 1;
                }
            }
            if value.is_empty() && !quoted {
                return Err(invalid("VDF token is empty."));
            }
            String::from_utf8(value).map_err(|_| invalid("Invalid VDF UTF-8."))
        }
        fn object(&mut self, depth: usize) -> Result<Vec<(String, VdfValue)>, FusionError> {
            if depth > DEPTH_LIMIT {
                return Err(invalid("VDF exceeds nesting limit."));
            }
            let mut result = Vec::new();
            loop {
                self.whitespace();
                match self.text.get(self.index) {
                    None if depth == 0 => return Ok(result),
                    Some(b'}') if depth > 0 => {
                        self.index += 1;
                        return Ok(result);
                    }
                    None | Some(b'}') => return Err(invalid("Unbalanced VDF object.")),
                    _ => {}
                }
                self.entries += 1;
                if self.entries > ENTRY_LIMIT {
                    return Err(invalid("VDF exceeds entry limit."));
                }
                let key = self.word()?;
                if key.is_empty() {
                    return Err(invalid("VDF key is empty."));
                }
                self.whitespace();
                let value = if self.text.get(self.index) == Some(&b'{') {
                    self.index += 1;
                    VdfValue::Object(self.object(depth + 1)?)
                } else {
                    VdfValue::Text(self.word()?)
                };
                result.push((key, value));
            }
        }
    }
    Parser {
        text: text.trim_start_matches('\u{feff}').as_bytes(),
        index: 0,
        entries: 0,
    }
    .object(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion_source::resolver::LocalLocatorResolver;
    use crate::fusion_source::schema::{CriticalFileRole, CriticalFileSpec, LocalLocatorSpec};

    fn escaped(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "\\\\")
    }
    fn scan_cancel() -> ScanCancel {
        ScanCancel {
            caller: Cancellation::default(),
            abort: Cancellation::default(),
        }
    }

    #[test]
    fn vdf_handles_comments_escapes_old_and_new_and_rejects_bounded_bad_input() {
        let parsed = parse_vdf("// comment\n\"libraryfolders\" { \"0\" \"C:\\\\Steam\" \"1\" { \"path\" \"D:\\\\Games\" } }").unwrap();
        let VdfValue::Object(entries) = lookup(&parsed, "libraryfolders").unwrap() else {
            panic!()
        };
        assert_eq!(entries.len(), 2);
        assert!(parse_vdf("\"AppState\" { \"empty\" \"\" }").is_ok());
        assert!(parse_vdf("\"broken\" { \"key\" \"value\"").is_err());
        assert!(parse_vdf(&format!("{}{}", "\"nested\" {".repeat(33), "}".repeat(33))).is_err());
        assert!(parse_vdf(&"x".repeat(FILE_LIMIT as usize + 1)).is_err());
        assert!(parse_vdf(&"\"key\" \"value\" ".repeat(ENTRY_LIMIT + 1)).is_err());
    }

    #[tokio::test]
    async fn steam_discovers_old_new_libraries_and_resolver_hashes_without_hints() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("Steam");
        let second = directory.path().join("OtherVolume");
        let third = directory.path().join("OldLibrary");
        let game = second.join("steamapps/common/ActualGame");
        for path in [
            &root.join("steamapps/common/Decoy"),
            &game,
            &third.join("steamapps/common/OldGame"),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} \"2\" \"{}\" }}",
                escaped(&second),
                escaped(&third)
            ),
        )
        .unwrap();
        fs::write(
            second.join("steamapps/appmanifest_42.acf"),
            "\"AppState\" { \"appid\" \"42\" \"installdir\" \"ActualGame\" }",
        )
        .unwrap();
        fs::write(root.join("steamapps/appmanifest_bad.acf"), "broken {").unwrap();
        fs::write(game.join("main.exe"), b"verified game").unwrap();
        let inventory = Arc::new(WindowsStorefrontInventory::with_roots(StorefrontRoots {
            steam_roots: vec![root],
            ..Default::default()
        }));
        let candidates = inventory
            .candidates(LocalStore::Steam, Some("42"), &Cancellation::default())
            .await
            .unwrap();
        assert!(candidates.contains(&game));
        assert!(candidates.contains(&third.join("steamapps/common/OldGame")));
        let library = directory.path().join("Fusion");
        fs::create_dir(&library).unwrap();
        let locator = LocalLocatorSpec {
            store: LocalStore::Steam,
            app_id: Some("42".into()),
            path_hints: Vec::new(),
            key_files: vec![CriticalFileSpec {
                path: "main.exe".into(),
                blake3: blake3::hash(b"verified game").to_hex().to_string(),
                role: CriticalFileRole::MainExecutable,
            }],
        };
        let resolver = LocalLocatorResolver::new().with_inventory(inventory);
        let found = resolver
            .resolve(&locator, &library, &Cancellation::default())
            .await
            .unwrap();
        assert_eq!(found.root, game);
        assert_eq!(
            found.volume_id,
            fs_safety::file_identity(&game).unwrap().volume.to_string()
        );
        assert_eq!(
            found.library_volume_id,
            fs_safety::file_identity(&library)
                .unwrap()
                .volume
                .to_string()
        );
        fs::write(game.join("main.exe"), b"changed game").unwrap();
        assert_eq!(
            resolver
                .resolve(&locator, &library, &Cancellation::default())
                .await
                .unwrap_err()
                .code,
            ErrorCode::LocalBaseModified
        );
    }

    #[tokio::test]
    async fn epic_gog_skip_bad_oversize_and_filter_ids_and_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let manifests = directory.path().join("Manifests");
        fs::create_dir(&manifests).unwrap();
        let game = directory.path().join("Game");
        fs::create_dir(&game).unwrap();
        fs::write(
            manifests.join("good.item"),
            serde_json::json!({"AppName":"abc", "InstallLocation":game}).to_string(),
        )
        .unwrap();
        fs::write(manifests.join("broken.item"), "{").unwrap();
        fs::write(
            manifests.join("huge.item"),
            vec![b' '; FILE_LIMIT as usize + 1],
        )
        .unwrap();
        let inventory = WindowsStorefrontInventory::with_roots(StorefrontRoots {
            epic_manifests: Some(manifests),
            gog_games: vec![
                (Some("123".into()), game.clone()),
                (None, PathBuf::from("relative")),
            ],
            ..Default::default()
        });
        assert_eq!(
            inventory
                .candidates(LocalStore::Epic, Some("abc"), &Cancellation::default())
                .await
                .unwrap(),
            vec![game.clone()]
        );
        assert!(inventory
            .candidates(LocalStore::Epic, Some("wrong"), &Cancellation::default())
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            inventory
                .candidates(LocalStore::Gog, Some("123"), &Cancellation::default())
                .await
                .unwrap(),
            vec![game]
        );
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            inventory
                .candidates(LocalStore::Steam, None, &cancel)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert_eq!(
            read_bounded(
                &directory.path().join("missing"),
                &ScanCancel {
                    caller: cancel,
                    ..scan_cancel()
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::Cancelled
        );
    }

    struct FixtureRegistry {
        steam: PathBuf,
        gog: PathBuf,
    }
    impl StorefrontRegistry for FixtureRegistry {
        fn steam_roots(&self, cancel: &Cancellation) -> Result<Vec<PathBuf>, FusionError> {
            cancel.check()?;
            Ok(vec![self.steam.clone()])
        }
        fn gog_games(
            &self,
            cancel: &Cancellation,
        ) -> Result<Vec<(Option<String>, PathBuf)>, FusionError> {
            cancel.check()?;
            Ok(vec![(Some("fixture-id".into()), self.gog.clone())])
        }
    }

    #[tokio::test]
    async fn registry_candidates_precede_fallbacks_and_inventory_stays_shallow() {
        let directory = tempfile::tempdir().unwrap();
        let steam = directory.path().join("Steam");
        let candidate = steam.join("steamapps/common/Game");
        let nested = candidate.join("NestedGame");
        fs::create_dir_all(&nested).unwrap();
        let gog = directory.path().join("GogGame");
        fs::create_dir(&gog).unwrap();
        let inventory = WindowsStorefrontInventory::with_roots(StorefrontRoots::default())
            .with_registry(Arc::new(FixtureRegistry {
                steam,
                gog: gog.clone(),
            }));
        assert_eq!(
            inventory
                .candidates(LocalStore::Steam, None, &Cancellation::default())
                .await
                .unwrap(),
            vec![candidate]
        );
        assert_eq!(
            inventory
                .candidates(
                    LocalStore::Gog,
                    Some("fixture-id"),
                    &Cancellation::default()
                )
                .await
                .unwrap(),
            vec![gog]
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn inventory_skips_junctions_even_when_registry_points_to_them() {
        use std::os::windows::process::CommandExt;
        let directory = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let junction = directory.path().join("GameJunction");
        let result = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(external.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(result.status.success());
        let inventory = WindowsStorefrontInventory::with_roots(StorefrontRoots {
            gog_games: vec![(Some("123".into()), junction.clone())],
            ..Default::default()
        });
        assert!(inventory
            .candidates(LocalStore::Gog, Some("123"), &Cancellation::default())
            .await
            .unwrap()
            .is_empty());
        fs::remove_dir(junction).unwrap();
    }
}
