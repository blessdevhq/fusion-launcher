//! Crash reports cross the privacy boundary here, before serialization or storage.
//! Raw logs and path context are deliberately not serializable. This module has
//! no network client; exporting a report requires a separate, user-driven adapter.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

use super::canonical::QualifiedGameId;
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::layers::LayerInput;
use super::schema::{LayerKind, RuntimeChannel};
use super::workspace::WorkspaceSnapshot;

#[derive(Clone, Debug, Default)]
pub struct TelemetryContext {
    pub os_username: Option<String>,
    pub workspace_root: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
    pub user_profile: Option<PathBuf>,
    pub local_base_roots: Vec<PathBuf>,
}

impl TelemetryContext {
    /// Environment values are used only to remove private text. They never
    /// become report fields or filesystem destinations.
    pub fn from_snapshot(snapshot: &WorkspaceSnapshot) -> Self {
        Self {
            os_username: std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .ok(),
            workspace_root: Some(snapshot.workspace_root.clone()),
            appdata: std::env::var_os("APPDATA").map(PathBuf::from),
            local_appdata: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            user_profile: std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from),
            local_base_roots: snapshot
                .inputs
                .values()
                .filter_map(|input| match input {
                    LayerInput::LocalDirectory { root } => Some(root.clone()),
                    LayerInput::Cas { .. } => None,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LogSanitizationLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_lines: usize,
}

impl Default for LogSanitizationLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 256 * 1024,
            max_output_bytes: 64 * 1024,
            max_lines: 200,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TelemetrySanitizer {
    replacements: Vec<(Regex, &'static str)>,
    username: Option<Regex>,
    limits: LogSanitizationLimits,
}

/// Only the sanitizer can produce this value. It is not a raw-log container.
#[derive(Clone, Debug)]
pub struct SanitizedLogTail(String);

impl SanitizedLogTail {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TelemetrySanitizer {
    pub fn new(
        context: TelemetryContext,
        limits: LogSanitizationLimits,
    ) -> Result<Self, FusionError> {
        if limits.max_input_bytes == 0
            || limits.max_input_bytes > 1024 * 1024
            || limits.max_output_bytes == 0
            || limits.max_output_bytes > 1024 * 1024
            || limits.max_lines == 0
            || limits.max_lines > 10_000
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Crash-log bounds must be nonzero and within the local memory budget.",
            ));
        }
        let mut paths = Vec::new();
        for (path, marker) in [
            (context.workspace_root, "<WORKSPACE_ROOT>"),
            (context.appdata, "<APPDATA>"),
            (context.local_appdata, "<LOCALAPPDATA>"),
            (context.user_profile, "<USER_PROFILE>"),
        ] {
            if let Some(path) = path {
                paths.push((path.to_string_lossy().into_owned(), marker));
            }
        }
        paths.extend(
            context
                .local_base_roots
                .into_iter()
                .map(|path| (path.to_string_lossy().into_owned(), "<LOCAL_BASE_ROOT>")),
        );
        // Match specific subdirectories before a shared user-profile prefix.
        paths.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        let replacements = paths
            .into_iter()
            .filter_map(|(path, marker)| path_expression(&path).map(|pattern| (pattern, marker)))
            .collect();
        let username = context
            .os_username
            .filter(|name| !name.trim().is_empty())
            .and_then(|name| Regex::new(&format!(
                "(?P<marker><(?:WORKSPACE_ROOT|APPDATA|LOCALAPPDATA|USER_PROFILE|LOCAL_BASE_ROOT|PERSISTENCE_ROOT|TEMP_ROOT|USER|EMAIL|IP|TOKEN|REDACTED)>)|(?i:{})",
                regex::escape(&name))).ok());
        Ok(Self {
            replacements,
            username,
            limits,
        })
    }

    pub fn for_snapshot(snapshot: &WorkspaceSnapshot) -> Self {
        let mut sanitizer = Self::new(
            TelemetryContext::from_snapshot(snapshot),
            LogSanitizationLimits::default(),
        )
        .expect("constant log limits are valid");
        let mut extra = Vec::new();
        for (key, value) in &snapshot.persistence.env_overrides {
            let marker = if key.eq_ignore_ascii_case("LOCALAPPDATA") {
                "<LOCALAPPDATA>"
            } else if key.eq_ignore_ascii_case("APPDATA") {
                "<APPDATA>"
            } else {
                "<PERSISTENCE_ROOT>"
            };
            if let Some(expression) = path_expression(value) {
                extra.push((expression, marker));
            }
        }
        for binding in &snapshot.persistence.bindings {
            if let Some(expression) = path_expression(&binding.persistent_path.to_string_lossy()) {
                extra.push((expression, "<PERSISTENCE_ROOT>"));
            }
        }
        // Persistent roots can be outside both workspace and the OS profile.
        // Redact these before shorter original context roots.
        extra.append(&mut sanitizer.replacements);
        sanitizer.replacements = extra;
        sanitizer
    }

    /// Mandatory privacy includes actual process paths even without APPDATA
    /// persistence declarations. Host-specific sanitizers may only add redaction.
    pub fn for_snapshot_environment(
        snapshot: &WorkspaceSnapshot,
        environment: &BTreeMap<String, String>,
    ) -> Self {
        let mut sanitizer = Self::for_snapshot(snapshot);
        for (name, marker) in [
            ("APPDATA", "<APPDATA>"),
            ("LOCALAPPDATA", "<LOCALAPPDATA>"),
            ("TEMP", "<TEMP_ROOT>"),
            ("TMP", "<TEMP_ROOT>"),
        ] {
            if let Some(pattern) = environment.get(name).and_then(|path| path_expression(path)) {
                sanitizer.replacements.push((pattern, marker));
            }
        }
        sanitizer
            .replacements
            .sort_by_key(|item| std::cmp::Reverse(item.0.as_str().len()));
        sanitizer
    }

    pub fn sanitize_logs(&self, raw: &str) -> SanitizedLogTail {
        let bounded = complete_line_tail(raw, self.limits.max_input_bytes);
        let selected = bounded
            .lines()
            .rev()
            .take(self.limits.max_lines)
            .collect::<Vec<_>>();
        let selected = selected.into_iter().rev().collect::<Vec<_>>().join("\n");
        let rules = rules();
        let mut clean = rules.ansi.replace_all(&selected, "").into_owned();
        clean.retain(|character| !character.is_control() || matches!(character, '\n' | '\t'));
        for (pattern, marker) in &self.replacements {
            clean = pattern.replace_all(&clean, *marker).into_owned();
        }
        clean = rules
            .bearer
            .replace_all(&clean, "$1 <REDACTED>")
            .into_owned();
        clean = rules
            .secret
            .replace_all(&clean, "$1=<REDACTED>")
            .into_owned();
        clean = rules.jwt.replace_all(&clean, "<TOKEN>").into_owned();
        clean = rules.email.replace_all(&clean, "<EMAIL>").into_owned();
        if let Some(username) = &self.username {
            clean = username
                .replace_all(&clean, |capture: &Captures<'_>| {
                    if capture.name("marker").is_some() {
                        capture[0].to_owned()
                    } else {
                        "<USER>".to_owned()
                    }
                })
                .into_owned();
        }
        clean = rules
            .ipv6
            .replace_all(&clean, |capture: &Captures<'_>| {
                let candidate = &capture[0];
                let address = if let Some(bracketed) = candidate.strip_prefix('[') {
                    bracketed.split(']').next().unwrap_or(bracketed)
                } else {
                    candidate
                };
                let address = address.trim_end_matches('.');
                let address = address.split('%').next().unwrap_or(address);
                let bare_with_port = address.rsplit_once(':').is_some_and(|(prefix, port)| {
                    port.parse::<u16>().is_ok() && prefix.parse::<Ipv6Addr>().is_ok()
                });
                if address.parse::<Ipv6Addr>().is_ok() || bare_with_port {
                    "<IP>".to_owned()
                } else {
                    candidate.to_owned()
                }
            })
            .into_owned();
        clean = rules
            .ipv4
            .replace_all(&clean, |capture: &Captures<'_>| {
                let candidate = &capture[0];
                if candidate
                    .split(':')
                    .next()
                    .unwrap_or(candidate)
                    .parse::<Ipv4Addr>()
                    .is_ok()
                {
                    "<IP>".to_owned()
                } else {
                    candidate.to_owned()
                }
            })
            .into_owned();
        SanitizedLogTail(utf8_tail(&clean, self.limits.max_output_bytes).to_owned())
    }
}

fn path_expression(path: &str) -> Option<Regex> {
    // A raw Windows path and its JSON-escaped form have interchangeable slash
    // runs. Matching is case insensitive, including mixed slash separators.
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = path.trim_end_matches(['\\', '/']);
    if path.is_empty() {
        return None;
    }
    let mut pattern = String::from("(?i)");
    let mut separator = false;
    for character in path.chars() {
        if matches!(character, '\\' | '/') {
            if !separator {
                pattern.push_str(r"[\\/]+");
            }
            separator = true;
        } else {
            separator = false;
            pattern.push_str(&regex::escape(&character.to_string()));
        }
    }
    Regex::new(&pattern).ok()
}

fn utf8_tail(value: &str, bytes: usize) -> &str {
    let mut start = value.len().saturating_sub(bytes);
    while !value.is_char_boundary(start) {
        start += 1;
    }
    &value[start..]
}

fn complete_line_tail(value: &str, bytes: usize) -> &str {
    if value.len() <= bytes {
        return value;
    }
    // Never expose the suffix of a truncated token/path without its identifying
    // prefix. Drop the first partial line entirely before sanitizing.
    let tail = utf8_tail(value, bytes);
    tail.find('\n').map_or("", |newline| &tail[newline + 1..])
}

struct SanitizationRules {
    ansi: Regex,
    secret: Regex,
    bearer: Regex,
    jwt: Regex,
    email: Regex,
    ipv4: Regex,
    ipv6: Regex,
    timestamp: Regex,
    memory_address: Regex,
    whitespace: Regex,
}

fn rules() -> &'static SanitizationRules {
    static RULES: OnceLock<SanitizationRules> = OnceLock::new();
    RULES.get_or_init(|| SanitizationRules {
        ansi: Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))").unwrap(),
        secret: Regex::new(
            r#"(?i)\b(token|access[_-]?token|refresh[_-]?token|id[_-]?token|session(?:[_-]?(?:id|token|key))?|api[_-]?key|client[_-]?secret|auth[_-]?token|authorization|password|passwd|secret|jwt|cookie)\b[\s"']*[:=]\s*(?:"[^"]*"|'[^']*'|[^\s,;}\]]+)"#,
        ).unwrap(),
        bearer: Regex::new(r"(?i)\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+").unwrap(),
        jwt: Regex::new(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b").unwrap(),
        email: Regex::new(r"(?i)[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?\.[a-z0-9-]+").unwrap(),
        ipv4: Regex::new(r"\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}(?::[0-9]{1,5})?\b").unwrap(),
        ipv6: Regex::new(r"(?i)(?:\[[0-9a-f:.]+(?:%[a-z0-9_.-]+)?\](?::[0-9]{1,5})?|[0-9a-f:.]*:[0-9a-f:.]+(?:%[a-z0-9_.-]+)?)").unwrap(),
        timestamp: Regex::new(r"(?i)\b(?:[0-9]{4}[-/][0-9]{2}[-/][0-9]{2}[T ][0-9]{2}:[0-9]{2}:[0-9]{2}(?:[.,][0-9]+)?(?:Z|[+-][0-9]{2}:?[0-9]{2})?|[0-9]{2}:[0-9]{2}:[0-9]{2}(?:[.,][0-9]+)?)\b").unwrap(),
        memory_address: Regex::new(r"(?i)\b(?:0x[0-9a-f]+|[0-9a-f]{16})\b").unwrap(),
        whitespace: Regex::new(r"\s+").unwrap(),
    })
}

#[derive(Clone, Copy, Debug)]
pub struct CrashOutcome {
    pub primary_exit_code: i32,
    pub user_cancelled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedRuntimeIdentity {
    channel: RuntimeChannel,
    version: String,
    executable_blake3: String,
}

/// Fields remain private so callers cannot put raw log/path strings back into a
/// sanitized report. Deserialization is intentionally not supported.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedCrashReport {
    schema_version: String,
    game: QualifiedGameId,
    layer_hashes: BTreeMap<String, String>,
    runtime: Option<SanitizedRuntimeIdentity>,
    enabled_addons: Vec<String>,
    exit_code: i32,
    clean_log_tail: String,
    crash_signature_hash: String,
}

impl SanitizedCrashReport {
    pub fn crash_signature_hash(&self) -> &str {
        &self.crash_signature_hash
    }

    pub fn clean_log_tail(&self) -> &str {
        &self.clean_log_tail
    }

    pub fn game(&self) -> &QualifiedGameId {
        &self.game
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

pub struct CrashReportBuilder;

impl CrashReportBuilder {
    pub fn build(
        snapshot: &WorkspaceSnapshot,
        outcome: CrashOutcome,
        raw_log_tail: &str,
        sanitizer: &TelemetrySanitizer,
    ) -> Result<Option<SanitizedCrashReport>, FusionError> {
        if outcome.user_cancelled || outcome.primary_exit_code == 0 {
            return Ok(None);
        }
        validate_id(&snapshot.game.source_id)?;
        validate_id(&snapshot.game.game_id)?;
        if snapshot.manifest.schema_version != "1.0" {
            return Err(FusionError::new(
                ErrorCode::UnsupportedSchema,
                "The crash report schema is unsupported.",
            ));
        }
        let mut layer_hashes = BTreeMap::new();
        let mut enabled_addons = Vec::new();
        for id in &snapshot.plan.selected_layers {
            validate_id(id)?;
            let layer = snapshot.manifest.layers.get(id).ok_or_else(|| {
                FusionError::new(
                    ErrorCode::MissingLayer,
                    "Crash context references an absent layer.",
                )
            })?;
            if matches!(
                layer.kind,
                LayerKind::Addon | LayerKind::Mod | LayerKind::ShaderCache
            ) {
                enabled_addons.push(id.clone());
            }
            let input = snapshot.inputs.get(id).ok_or_else(|| {
                FusionError::new(
                    ErrorCode::MissingLayer,
                    "Crash context has no active layer binding.",
                )
            })?;
            let digest = match input {
                LayerInput::Cas { digest, .. } => {
                    validate_hash(digest)?;
                    digest.clone()
                }
                LayerInput::LocalDirectory { .. } => {
                    // Hash only the declared key-file identities, never the
                    // private discovered directory path.
                    let mut keys = layer
                        .critical_files
                        .iter()
                        .map(|file| &file.blake3)
                        .collect::<Vec<_>>();
                    if let super::schema::LayerSource::LocalLocator { locator } = &layer.source {
                        keys.extend(locator.key_files.iter().map(|file| &file.blake3));
                    }
                    keys.sort();
                    for digest in &keys {
                        validate_hash(digest)?;
                    }
                    let mut hasher = blake3::Hasher::new();
                    hasher.update(b"fusion-local-key-inventory-v1\0");
                    for digest in keys {
                        hasher.update(digest.as_bytes());
                        hasher.update(b"\0");
                    }
                    hasher.finalize().to_hex().to_string()
                }
            };
            layer_hashes.insert(id.clone(), digest);
        }
        enabled_addons.sort();
        let runtime = runtime_identity(snapshot)?;
        let clean_log_tail = sanitizer.sanitize_logs(raw_log_tail).0;
        let normalized = normalize_signature_log(&clean_log_tail);
        // JSON arrays bind field boundaries and avoid concatenation collisions.
        let signature = serde_json::to_vec(&(
            "fusion-crash-signature-v1",
            &snapshot.game,
            &runtime,
            outcome.primary_exit_code,
            normalized,
        ))
        .map_err(|_| {
            FusionError::new(ErrorCode::InvalidManifest, "Cannot encode crash identity.")
        })?;
        Ok(Some(SanitizedCrashReport {
            schema_version: "1.0".into(),
            game: snapshot.game.clone(),
            layer_hashes,
            runtime,
            enabled_addons,
            exit_code: outcome.primary_exit_code,
            clean_log_tail,
            crash_signature_hash: blake3::hash(&signature).to_hex().to_string(),
        }))
    }
}

fn runtime_identity(
    snapshot: &WorkspaceSnapshot,
) -> Result<Option<SanitizedRuntimeIdentity>, FusionError> {
    let Some(runtime) = &snapshot.manifest.runtime else {
        let primary = snapshot.manifest.launch.as_ref().or_else(|| {
            snapshot.manifest.orchestrator.as_ref().and_then(|graph| {
                graph
                    .processes
                    .iter()
                    .find(|process| process.role == super::schema::ProcessRole::Primary)
            })
        });
        let Some(primary) = primary else {
            return Ok(None);
        };
        let version = safe_version_identity(&snapshot.manifest.version)?;
        return Ok(Some(SanitizedRuntimeIdentity {
            channel: RuntimeChannel::Pinned,
            version,
            executable_blake3: final_executable_digest(snapshot, &primary.executable)?,
        }));
    };
    let input = snapshot.inputs.get(&runtime.layer_id).ok_or_else(|| {
        FusionError::new(
            ErrorCode::MissingLayer,
            "The crash runtime binding is absent.",
        )
    })?;
    let channel = snapshot
        .plan
        .runtime_channel
        .unwrap_or(RuntimeChannel::Pinned);
    let version = match (channel, input) {
        (
            RuntimeChannel::Dynamic,
            LayerInput::Cas {
                runtime_lock: Some(lock),
                ..
            },
        ) => lock.release_tag.as_ref().unwrap_or(&lock.release_id),
        (RuntimeChannel::Dynamic, _) => {
            return Err(FusionError::new(
                ErrorCode::InvalidRuntimeChannel,
                "A dynamic crash context requires its recorded runtime lock.",
            ));
        }
        (RuntimeChannel::Pinned, _) => &snapshot.manifest.version,
    };
    let version = safe_version_identity(version)?;
    Ok(Some(SanitizedRuntimeIdentity {
        channel,
        version,
        executable_blake3: final_executable_digest(snapshot, &runtime.executable)?,
    }))
}

fn final_executable_digest(
    snapshot: &WorkspaceSnapshot,
    executable: &str,
) -> Result<String, FusionError> {
    // Match the final overlay inventory used by the integrity guard, including
    // local keys, runtime locks and patched/replaced main executables.
    for id in snapshot.plan.selected_layers.iter().rev() {
        let layer = snapshot.manifest.layers.get(id).ok_or_else(|| {
            FusionError::new(
                ErrorCode::MissingLayer,
                "Crash context references an absent layer.",
            )
        })?;
        let declared = match snapshot.inputs.get(id) {
            Some(LayerInput::Cas {
                runtime_lock: Some(lock),
                ..
            }) => &lock.critical_files,
            _ => &layer.critical_files,
        };
        let keys = match &layer.source {
            super::schema::LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
            _ => &[],
        };
        if let Some(file) = keys.iter().chain(declared).rev().find(|file| {
            super::evolution::mounted_path(&layer.mount_path, &file.path)
                .eq_ignore_ascii_case(executable)
        }) {
            validate_hash(&file.blake3)?;
            return Ok(file.blake3.to_ascii_lowercase());
        }
    }
    Err(FusionError::new(
        ErrorCode::InvalidManifest,
        "Crash context has no final executable digest.",
    ))
}

fn validate_id(value: &str) -> Result<(), FusionError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidId,
            "Unsafe identifier in crash metadata.",
        ));
    }
    Ok(())
}

fn safe_version_identity(value: &str) -> Result<String, FusionError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Unsafe runtime version in crash metadata.",
        ));
    }
    if value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+'))
    {
        Ok(value.to_owned())
    } else {
        // Free-form display versions and slash-separated GitHub tags are valid
        // source metadata. Preserve their identity without exporting private text.
        Ok(format!(
            "redacted-{}",
            blake3::hash(value.as_bytes()).to_hex()
        ))
    }
}

fn validate_hash(value: &str) -> Result<(), FusionError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FusionError::new(
            ErrorCode::InvalidDigest,
            "Invalid digest in crash metadata.",
        ));
    }
    Ok(())
}

fn normalize_signature_log(value: &str) -> String {
    let rules = rules();
    let timestamp_free = rules.timestamp.replace_all(value, "<TIME>");
    let address_free = rules
        .memory_address
        .replace_all(&timestamp_free, "<ADDRESS>");
    rules
        .whitespace
        .replace_all(&address_free, " ")
        .trim()
        .to_owned()
}

/// A future adapter calls this only in response to the user's explicit report
/// action. This module never calls it, discovers endpoints or sends reports.
pub trait CrashReportExporter: Send + Sync {
    fn export_user_selected(&self, report: &SanitizedCrashReport) -> Result<(), FusionError>;
}

#[derive(Clone, Copy, Debug)]
pub struct DeduplicationPolicy {
    pub max_reports_per_signature: u32,
    pub cooldown_secs: u64,
    pub window_secs: u64,
    pub max_signatures: usize,
}

impl Default for DeduplicationPolicy {
    fn default() -> Self {
        Self {
            max_reports_per_signature: 3,
            cooldown_secs: 5 * 60,
            window_secs: 24 * 60 * 60,
            max_signatures: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeduplicationDecision {
    Accepted { remaining_in_window: u32 },
    SuppressedCooldown { retry_after_secs: u64 },
    SuppressedLimit { retry_after_secs: u64 },
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeduplicationState {
    entries: BTreeMap<String, DeduplicationEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeduplicationEntry {
    window_started: u64,
    last_accepted: u64,
    last_seen: u64,
    accepted_count: u32,
}

/// The ledger persists only signatures and numeric rate-limit state. It stores
/// no raw or sanitized logs, paths, usernames, endpoints or authorization data.
pub struct DeduplicationLedger {
    root: PathBuf,
    policy: DeduplicationPolicy,
}

impl DeduplicationLedger {
    pub fn open(owned_directory: &Path, policy: DeduplicationPolicy) -> Result<Self, FusionError> {
        if policy.max_reports_per_signature == 0
            || policy.max_reports_per_signature > 100
            || policy.window_secs == 0
            || policy.max_signatures == 0
            || policy.max_signatures > 2048
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Invalid crash deduplication bounds.",
            ));
        }
        let _root = fs_safety::ensure_directory(owned_directory)?;
        Ok(Self {
            root: owned_directory.to_owned(),
            policy,
        })
    }

    pub fn admit(
        &self,
        report: &SanitizedCrashReport,
        now_unix_secs: u64,
    ) -> Result<DeduplicationDecision, FusionError> {
        validate_hash(report.crash_signature_hash())?;
        let _guard = fs_safety::guard(&self.root)?;
        let _lock = self.lock()?;
        let mut state = self.read_state()?;
        state.entries.retain(|_, entry| {
            now_unix_secs.saturating_sub(entry.window_started) < self.policy.window_secs
        });
        let signature = report.crash_signature_hash();
        let limit =
            self.policy.max_signatures - usize::from(!state.entries.contains_key(signature));
        while state.entries.len() > limit {
            let oldest = state
                .entries
                .iter()
                .filter(|(key, _)| key.as_str() != signature)
                .min_by_key(|(signature, entry)| (entry.last_seen, *signature))
                .map(|(signature, _)| signature.clone());
            if let Some(oldest) = oldest {
                state.entries.remove(&oldest);
            } else {
                break;
            }
        }
        let entry = state
            .entries
            .entry(signature.to_owned())
            .or_insert(DeduplicationEntry {
                window_started: now_unix_secs,
                last_accepted: now_unix_secs,
                last_seen: now_unix_secs,
                accepted_count: 0,
            });
        entry.last_seen = now_unix_secs;
        let decision = if entry.accepted_count >= self.policy.max_reports_per_signature {
            DeduplicationDecision::SuppressedLimit {
                retry_after_secs: self
                    .policy
                    .window_secs
                    .saturating_sub(now_unix_secs.saturating_sub(entry.window_started)),
            }
        } else if entry.accepted_count > 0
            && now_unix_secs.saturating_sub(entry.last_accepted) < self.policy.cooldown_secs
        {
            DeduplicationDecision::SuppressedCooldown {
                retry_after_secs: self
                    .policy
                    .cooldown_secs
                    .saturating_sub(now_unix_secs.saturating_sub(entry.last_accepted)),
            }
        } else {
            entry.accepted_count += 1;
            entry.last_accepted = now_unix_secs;
            DeduplicationDecision::Accepted {
                remaining_in_window: self.policy.max_reports_per_signature - entry.accepted_count,
            }
        };
        fs_safety::atomic_write_json(&self.root.join("crash-dedup.json"), &state)?;
        Ok(decision)
    }

    fn read_state(&self) -> Result<DeduplicationState, FusionError> {
        let path = self.root.join("crash-dedup.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DeduplicationState::default())
            }
            Err(error) => return Err(fs_safety::io_error("Inspect crash ledger", &path, error)),
            Ok(metadata) => fs_safety::check_metadata(&metadata, &path)?,
        }
        let mut bytes = Vec::new();
        fs_safety::open_read(&path)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| fs_safety::io_error("Read crash ledger", &path, error))?;
        if bytes.len() > 1024 * 1024 {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Crash ledger exceeds its memory bound.",
            ));
        }
        let state: DeduplicationState = serde_json::from_slice(&bytes)
            .map_err(|_| FusionError::new(ErrorCode::InvalidJson, "Invalid local crash ledger."))?;
        if state.entries.len() > 2048 {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Crash ledger has too many signatures.",
            ));
        }
        for (signature, entry) in &state.entries {
            validate_hash(signature)?;
            if entry.accepted_count > 100 {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Invalid crash ledger count.",
                ));
            }
        }
        Ok(state)
    }

    fn lock(&self) -> Result<File, FusionError> {
        let path = self.root.join("crash-dedup.lock");
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            fs_safety::check_metadata(&metadata, &path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x00200000);
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0x20000);
        }
        let file = options
            .open(&path)
            .map_err(|error| fs_safety::io_error("Open crash ledger lock", &path, error))?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
            FusionError::new(
                ErrorCode::OperationInProgress,
                "Crash ledger is already being updated.",
            )
        })?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::*;
    use crate::fusion_source::capabilities::CapabilityRegistry;
    use crate::fusion_source::cas::CasStore;
    use crate::fusion_source::config_templates::TemplateContext;
    use crate::fusion_source::evolution::SchemaUpcaster;
    use crate::fusion_source::persistence::PersistenceRoots;
    use crate::fusion_source::plan::{PlanBuilder, PlanOptions};
    use crate::fusion_source::workspace::{
        AssembledWorkspace, WorkspaceAssembler, WorkspaceBuildOptions,
    };

    fn sanitizer() -> TelemetrySanitizer {
        TelemetrySanitizer::new(
            TelemetryContext {
                os_username: Some("Denis".into()),
                workspace_root: Some(PathBuf::from(r"C:\Fusion\workspace\build-123")),
                appdata: Some(PathBuf::from(r"C:\Users\Denis\AppData\Roaming")),
                local_appdata: Some(PathBuf::from(r"C:\Users\Denis\AppData\Local")),
                user_profile: Some(PathBuf::from(r"C:\Users\Denis")),
                local_base_roots: vec![PathBuf::from(
                    r"D:\SteamLibrary\steamapps\common\Cyberpunk 2077",
                )],
            },
            LogSanitizationLimits::default(),
        )
        .unwrap()
    }

    fn snapshot() -> (tempfile::TempDir, AssembledWorkspace) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let source = root.join("source");
        let library = root.join("library");
        let appdata = root.join("appdata");
        let local_appdata = root.join("local-appdata");
        for directory in [&source, &library, &appdata, &local_appdata] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(source.join("runtime.exe"), b"test runtime identity").unwrap();
        let digest = blake3::hash(b"test runtime identity").to_hex().to_string();
        let manifest = SchemaUpcaster::upcast_game_manifest("test-source", &json!({
            "schema_version":"1.0", "id":"telemetry-game", "title":"Telemetry fixture", "version":"1.2.3",
            "layers":[{"id":"runtime", "kind":"runtime", "source":{"kind":"user_provided"},
                "critical_files":[{"path":"runtime.exe", "blake3":digest, "role":"main_executable"}]}],
            "runtime":{"layer_id":"runtime","executable":"runtime.exe"}
        }).to_string(), &BTreeMap::new()).unwrap();
        let cas = CasStore::open(&library).unwrap();
        let object = cas.import_directory(&source).unwrap();
        let plan = PlanBuilder::build(
            &manifest,
            &PlanOptions::default(),
            &CapabilityRegistry::current_client().with("proc.supervisor.v1"),
        )
        .unwrap();
        let assembler = WorkspaceAssembler::open(
            &library,
            &QualifiedGameId {
                source_id: "test-source".into(),
                game_id: "telemetry-game".into(),
            },
        )
        .unwrap();
        let workspace = assembler
            .assemble(
                &manifest,
                &plan,
                &BTreeMap::from([(
                    "runtime".into(),
                    LayerInput::Cas {
                        digest: object.digest,
                        runtime_lock: None,
                    },
                )]),
                &TemplateContext {
                    workspace_dir: library.clone(),
                    appdata_dir: appdata.clone(),
                    local_appdata_dir: local_appdata.clone(),
                    display_width: 1920,
                    display_height: 1080,
                    refresh_rate: 60,
                    enabled_addons: BTreeSet::new(),
                },
                &PersistenceRoots {
                    appdata,
                    local_appdata,
                },
                &WorkspaceBuildOptions::default(),
            )
            .unwrap();
        (temporary, workspace)
    }

    fn report(snapshot: &WorkspaceSnapshot, logs: &str) -> SanitizedCrashReport {
        CrashReportBuilder::build(
            snapshot,
            CrashOutcome {
                primary_exit_code: 17,
                user_cancelled: false,
            },
            logs,
            &sanitizer(),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn dirty_logs_remove_private_paths_addresses_emails_and_tokens() {
        let raw = concat!(
            "Denis loaded C:\\Users\\Denis\\AppData\\Roaming\\Emulator\\config.ini\n",
            "cache c:/users/DENIS/AppData/Local/cache; profile C:\\Users\\Denis\\Documents\n",
            "workspace C:\\Fusion\\workspace\\build-123\\runtime.exe\n",
            "steam d:/SteamLibrary/steamapps/common/Cyberpunk 2077/bin/game.exe\n",
            "escaped C:\\\\Users\\\\Denis\\\\AppData\\\\Roaming\\\\settings.json\n",
            "mail denis.private+debug@personal.example.org user=Denis\n",
            "addresses 192.168.1.17:27801 203.0.113.42 [2001:db8::1234]:8888 fe80::abcd%eth0 ::1 ::ffff:192.0.2.1\n",
            "Authorization: Bearer bearer-secret-123\n",
            "api_key=secret-api-value session_id='secret session value' access_token:\"secret-access-value\"\n",
            "token=generic-secret-value client_secret=client-secret-value session_key=session-key-value\n",
            "Cookie: session=secret-cookie-value; Password=secret-password\n",
            "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signaturehere\n",
            "crash happened in renderer at 0x7FF12345678\n"
        );
        let sanitized = sanitizer().sanitize_logs(raw);
        let clean = sanitized.as_str();
        for private in [
            "denis",
            "c:\\users",
            "c:\\\\users",
            "c:\\fusion",
            "d:/steamlibrary",
            "personal.example.org",
            "192.168.1.17",
            "203.0.113.42",
            "2001:db8",
            "fe80::",
            "::1",
            "192.0.2.1",
            "bearer-secret",
            "secret-api",
            "secret session",
            "secret-access",
            "generic-secret-value",
            "client-secret-value",
            "session-key-value",
            "secret-cookie",
            "secret-password",
            "eyJhbGci",
        ] {
            assert!(
                !clean.to_lowercase().contains(private),
                "private substring {private} survived: {clean}"
            );
        }
        for marker in [
            "<USER>",
            "<WORKSPACE_ROOT>",
            "<APPDATA>",
            "<LOCALAPPDATA>",
            "<USER_PROFILE>",
            "<LOCAL_BASE_ROOT>",
            "<EMAIL>",
            "<IP>",
            "<REDACTED>",
        ] {
            assert!(
                clean.contains(marker),
                "expected privacy marker {marker}: {clean}"
            );
        }
        assert!(clean.contains("crash happened in renderer"));
        let default_username = TelemetrySanitizer::new(
            TelemetryContext {
                os_username: Some("User".into()),
                user_profile: Some(PathBuf::from(r"C:\Users\User")),
                ..TelemetryContext::default()
            },
            LogSanitizationLimits::default(),
        )
        .unwrap();
        let once = default_username.sanitize_logs(r"User loaded C:\Users\User\game.cfg");
        assert!(once.as_str().contains("<USER_PROFILE>"));
        assert_eq!(
            once.as_str(),
            default_username.sanitize_logs(once.as_str()).as_str()
        );
    }

    #[test]
    fn log_bounds_drop_partial_secret_lines_and_preserve_utf8() {
        let bounded = TelemetrySanitizer::new(
            TelemetryContext::default(),
            LogSanitizationLimits {
                max_input_bytes: 32,
                max_output_bytes: 16,
                max_lines: 2,
            },
        )
        .unwrap();
        assert_eq!(
            bounded
                .sanitize_logs(&format!("token={}", "private".repeat(100)))
                .as_str(),
            ""
        );
        let clean = bounded.sanitize_logs("ignore\nold\nпривет\nlatest 😀");
        assert!(clean.as_str().len() <= 16);
        assert!(clean.as_str().ends_with("latest 😀"));
        assert!(!clean.as_str().contains("old"));
        assert!(TelemetrySanitizer::new(
            TelemetryContext::default(),
            LogSanitizationLimits {
                max_input_bytes: usize::MAX,
                ..LogSanitizationLimits::default()
            }
        )
        .is_err());
    }

    #[test]
    fn same_crash_with_different_timestamps_and_memory_addresses_has_same_signature() {
        let (_temporary, lease) = snapshot();
        let first = report(&lease.snapshot, "[2026-10-16T12:00:01.123Z] renderer crashed at 0x7FF12345678\n12:01:02.111 failed frame");
        let second = report(&lease.snapshot, "[2026-11-20T23:59:59.999Z] renderer crashed at 0x6AA87654321\n23:59:59.999 failed frame");
        assert_eq!(first.crash_signature_hash(), second.crash_signature_hash());
        let windows_first = report(
            &lease.snapshot,
            "ExceptionAddress: 00007FF6D28B1234 renderer crashed",
        );
        let windows_second = report(
            &lease.snapshot,
            "ExceptionAddress: 00007FF6FA9C5678 renderer crashed",
        );
        assert_eq!(
            windows_first.crash_signature_hash(),
            windows_second.crash_signature_hash()
        );
        let distinct = report(
            &lease.snapshot,
            "[2026-10-16T12:00:01.123Z] audio crashed at 0x7FF12345678",
        );
        assert_ne!(
            first.crash_signature_hash(),
            distinct.crash_signature_hash()
        );
        let mut different_runtime = lease.snapshot.clone();
        different_runtime.manifest.version = "1.2.4".into();
        assert_ne!(
            first.crash_signature_hash(),
            report(&different_runtime, first.clean_log_tail()).crash_signature_hash()
        );
    }

    #[test]
    fn report_contains_only_safe_technical_metadata_and_rejects_injection() {
        let (_temporary, lease) = snapshot();
        let raw = r"C:\Users\Denis\AppData\Roaming\config.ini Authorization: Bearer supersecret";
        let report = report(&lease.snapshot, raw);
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains("Denis"));
        assert!(!serialized.contains("supersecret"));
        assert!(!serialized.contains(&lease.snapshot.workspace_root.to_string_lossy().to_string()));
        let object = serde_json::to_value(report).unwrap();
        let keys = object
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "cleanLogTail",
                "crashSignatureHash",
                "enabledAddons",
                "exitCode",
                "game",
                "layerHashes",
                "runtime",
                "schemaVersion"
            ]
        );
        let mut invalid = lease.snapshot.clone();
        invalid.game.source_id = "C:\\Users\\Denis\\private".into();
        assert_eq!(
            CrashReportBuilder::build(
                &invalid,
                CrashOutcome {
                    primary_exit_code: 17,
                    user_cancelled: false
                },
                "failure",
                &sanitizer()
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidId
        );
        invalid = lease.snapshot.clone();
        invalid.manifest.version = "v1\nAuthorization: secret".into();
        assert!(CrashReportBuilder::build(
            &invalid,
            CrashOutcome {
                primary_exit_code: 17,
                user_cancelled: false
            },
            "failure",
            &sanitizer()
        )
        .is_err());
    }

    #[test]
    fn free_form_runtime_versions_remain_reportable_without_exporting_raw_text() {
        let (_temporary, lease) = snapshot();
        let mut snapshot = lease.snapshot.clone();
        snapshot.manifest.version = "Nightly 2026/10/16 сборка".into();
        let first = report(&snapshot, "renderer crashed");
        let second = report(&snapshot, "renderer crashed");
        let serialized = serde_json::to_string(&first).unwrap();
        assert!(serialized.contains("redacted-"));
        assert!(!serialized.contains("Nightly 2026"));
        assert!(!serialized.contains("сборка"));
        assert_eq!(first.crash_signature_hash(), second.crash_signature_hash());
        snapshot.manifest.version = "nightly/2026-10-17".into();
        assert_ne!(
            first.crash_signature_hash(),
            report(&snapshot, "renderer crashed").crash_signature_hash()
        );
    }

    #[test]
    fn dynamic_identity_includes_locked_version_and_hash_without_asset_urls() {
        let (_temporary, lease) = snapshot();
        let mut dynamic = lease.snapshot.clone();
        dynamic.plan.runtime_channel = Some(RuntimeChannel::Dynamic);
        let critical = dynamic.manifest.layers["runtime"].critical_files.clone();
        if let LayerInput::Cas { runtime_lock, .. } = dynamic.inputs.get_mut("runtime").unwrap() {
            *runtime_lock = Some(super::super::layers::ResolvedRuntimeLock {
                release_id: "998".into(),
                asset_id: "777".into(),
                asset_url: Some(
                    "https://example.org/private-asset?token=private-download-token".into(),
                ),
                release_tag: Some("v3.0.0-nightly".into()),
                transport_digest: super::super::schema::DigestSpec {
                    algorithm: super::super::schema::DigestAlgorithm::Blake3,
                    value: "a".repeat(64),
                },
                critical_files: critical,
                tool_versions: BTreeMap::new(),
            });
        }
        let dynamic_report = report(&dynamic, "runtime crash");
        let serialized = serde_json::to_string(&dynamic_report).unwrap();
        assert!(serialized.contains("v3.0.0-nightly"));
        assert!(serialized.contains("dynamic"));
        assert!(!serialized.contains("private-asset"));
        assert!(!serialized.contains("private-download-token"));
        assert_ne!(
            dynamic_report.crash_signature_hash(),
            report(&lease.snapshot, "runtime crash").crash_signature_hash()
        );
    }

    #[test]
    fn snapshot_default_also_redacts_isolated_persistence_roots() {
        let (_temporary, lease) = snapshot();
        let mut snapshot = lease.snapshot.clone();
        snapshot.persistence.env_overrides.insert(
            "APPDATA".into(),
            r"D:\PrivateLibrary\isolated-appdata".into(),
        );
        snapshot.persistence.env_overrides.insert(
            "LOCALAPPDATA".into(),
            r"D:\PrivateLibrary\isolated-local-appdata".into(),
        );
        let sanitizer = TelemetrySanitizer::for_snapshot(&snapshot);
        let clean = sanitizer.sanitize_logs(r"D:\PrivateLibrary\isolated-appdata\settings D:\PrivateLibrary\isolated-local-appdata\cache");
        assert!(!clean.as_str().contains("PrivateLibrary"));
        assert!(clean.as_str().contains("<APPDATA>"));
        assert!(clean.as_str().contains("<LOCALAPPDATA>"));
    }

    #[test]
    fn cancelled_and_successful_processes_do_not_produce_crash_reports() {
        let (_temporary, lease) = snapshot();
        for outcome in [
            CrashOutcome {
                primary_exit_code: 0,
                user_cancelled: false,
            },
            CrashOutcome {
                primary_exit_code: 17,
                user_cancelled: true,
            },
        ] {
            assert!(CrashReportBuilder::build(
                &lease.snapshot,
                outcome,
                "dirty private log",
                &sanitizer()
            )
            .unwrap()
            .is_none());
        }
    }

    #[test]
    fn deduplication_persists_cooldown_repeat_limit_and_bounded_state() {
        let (temporary, lease) = snapshot();
        let report = report(&lease.snapshot, "renderer failure");
        let directory = temporary.path().join("sanitized-report-ledger");
        let policy = DeduplicationPolicy {
            max_reports_per_signature: 2,
            cooldown_secs: 10,
            window_secs: 60,
            max_signatures: 2,
        };
        let ledger = DeduplicationLedger::open(&directory, policy).unwrap();
        assert_eq!(
            ledger.admit(&report, 100).unwrap(),
            DeduplicationDecision::Accepted {
                remaining_in_window: 1
            }
        );
        assert_eq!(
            ledger.admit(&report, 105).unwrap(),
            DeduplicationDecision::SuppressedCooldown {
                retry_after_secs: 5
            }
        );
        drop(ledger);
        let restarted = DeduplicationLedger::open(&directory, policy).unwrap();
        assert_eq!(
            restarted.admit(&report, 110).unwrap(),
            DeduplicationDecision::Accepted {
                remaining_in_window: 0
            }
        );
        assert_eq!(
            restarted.admit(&report, 150).unwrap(),
            DeduplicationDecision::SuppressedLimit {
                retry_after_secs: 10
            }
        );
        assert_eq!(
            restarted.admit(&report, 160).unwrap(),
            DeduplicationDecision::Accepted {
                remaining_in_window: 1
            }
        );
        for index in 0..20 {
            let distinct = self::report(&lease.snapshot, &format!("distinct failure {index}"));
            restarted.admit(&distinct, 161 + index).unwrap();
        }
        let raw = fs::read_to_string(directory.join("crash-dedup.json")).unwrap();
        assert!(!raw.contains("renderer"));
        assert!(!raw.contains("distinct failure"));
        assert!(!raw.contains("workspace"));
        let stored: DeduplicationState = serde_json::from_str(&raw).unwrap();
        assert_eq!(stored.entries.len(), 2);
        assert!(raw.len() < 1024);
    }

    #[test]
    fn oversized_or_private_ledger_input_fails_closed() {
        let (temporary, lease) = snapshot();
        let directory = temporary.path().join("ledger");
        let ledger = DeduplicationLedger::open(&directory, DeduplicationPolicy::default()).unwrap();
        fs::write(
            directory.join("crash-dedup.json"),
            vec![b' '; 1024 * 1024 + 1],
        )
        .unwrap();
        assert!(ledger
            .admit(&report(&lease.snapshot, "failure"), 100)
            .is_err());
        fs::write(directory.join("crash-dedup.json"), br#"{"entries":{"C:\\Users\\Denis":{"windowStarted":0,"lastAccepted":0,"lastSeen":0,"acceptedCount":1}}}"#).unwrap();
        assert_eq!(
            ledger
                .admit(&report(&lease.snapshot, "failure"), 100)
                .unwrap_err()
                .code,
            ErrorCode::InvalidDigest
        );
    }
}
