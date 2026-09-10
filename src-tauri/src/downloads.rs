use std::collections::HashMap;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::RANGE;
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::builtin_demo;
use crate::schema::SourceUri;

/// Connection timeout for HTTP downloads. Intentionally no overall request
/// timeout: large game/emulator archives can legitimately take a long time.
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Some hosts (notably the GitHub API/redirect chain) reject requests without a
/// User-Agent, so every download identifies itself.
const DOWNLOAD_USER_AGENT: &str = concat!("FusionLauncher/", env!("CARGO_PKG_VERSION"));

/// Minimum gap between streaming progress callbacks, to avoid flooding the IPC
/// channel and SQLite on fast connections.
const PROGRESS_REPORT_INTERVAL: Duration = Duration::from_millis(400);
pub const DOWNLOAD_CANCELLED_MESSAGE: &str = "Download cancelled.";

#[derive(Debug)]
pub struct DownloadedFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Default)]
pub struct DirectDownloadRegistry {
    tokens: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[derive(Clone)]
pub struct DirectDownloadCancelToken {
    token: Arc<AtomicBool>,
}

pub struct DirectDownloadGuard {
    id: String,
    token: Arc<AtomicBool>,
    registry: DirectDownloadRegistry,
}

impl DirectDownloadRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, id: &str) -> Result<DirectDownloadGuard, String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("Download id cannot be empty.".to_string());
        }

        let token = Arc::new(AtomicBool::new(false));
        let mut tokens = self.lock_tokens()?;
        if tokens.contains_key(id) {
            return Err(format!("Download is already active: {id}"));
        }
        tokens.insert(id.to_string(), Arc::clone(&token));

        Ok(DirectDownloadGuard {
            id: id.to_string(),
            token,
            registry: self.clone(),
        })
    }

    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        let Some(token) = self.lock_tokens()?.get(id).cloned() else {
            return Ok(false);
        };
        token.store(true, Ordering::SeqCst);
        Ok(true)
    }

    fn finish(&self, id: &str, token: &Arc<AtomicBool>) {
        let Ok(mut tokens) = self.tokens.lock() else {
            return;
        };
        if tokens
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, token))
        {
            tokens.remove(id);
        }
    }

    fn lock_tokens(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>, String> {
        self.tokens
            .lock()
            .map_err(|_| "Direct download registry lock is poisoned.".to_string())
    }
}

impl DirectDownloadGuard {
    pub fn token(&self) -> DirectDownloadCancelToken {
        DirectDownloadCancelToken {
            token: Arc::clone(&self.token),
        }
    }
}

impl Drop for DirectDownloadGuard {
    fn drop(&mut self) {
        self.registry.finish(&self.id, &self.token);
    }
}

impl DirectDownloadCancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.token.load(Ordering::SeqCst)
    }
}

pub fn is_download_cancelled_error(error: &str) -> bool {
    error == DOWNLOAD_CANCELLED_MESSAGE
}

pub(crate) struct DirectDownloadSpeedMeter {
    previous: Option<(u64, Instant)>,
}

impl DirectDownloadSpeedMeter {
    pub(crate) fn new() -> Self {
        Self { previous: None }
    }

    pub(crate) fn sample(&mut self, downloaded_bytes: u64) -> u64 {
        self.sample_at(downloaded_bytes, Instant::now())
    }

    fn sample_at(&mut self, downloaded_bytes: u64, now: Instant) -> u64 {
        let Some((previous_bytes, previous_at)) = self.previous.replace((downloaded_bytes, now))
        else {
            return 0;
        };

        if downloaded_bytes <= previous_bytes {
            return 0;
        }

        let elapsed = now.saturating_duration_since(previous_at);
        if elapsed.is_zero() {
            return 0;
        }

        ((downloaded_bytes - previous_bytes) as f64 / elapsed.as_secs_f64()).round() as u64
    }
}

/// Options controlling a streaming HTTP download.
pub struct StreamOptions<'a> {
    /// Expected SHA-256 of the full file. Verified after streaming completes.
    /// `None` skips verification (e.g. GithubLatest emulator assets without a
    /// pinned hash); callers that skip verification must trust the source.
    pub expected_sha256: Option<&'a str>,
    /// Expected total size in bytes, verified after completion when present.
    pub expected_size_bytes: Option<u64>,
    /// Hard cap; the stream is aborted early (without buffering the whole body)
    /// once this many bytes are written. `None` means no cap.
    pub max_bytes: Option<u64>,
    /// When true, an existing `.part` file is resumed via a `Range` request and
    /// survives across app restarts. When false, any existing `.part` is
    /// discarded before downloading.
    pub resume: bool,
}

pub async fn download_source_to_file(
    source: &SourceUri,
    destination: &Path,
) -> Result<DownloadedFile, String> {
    download_source_to_file_with_progress(source, destination, |_, _| {}).await
}

/// Like [`download_source_to_file`], but reports streaming progress
/// (`downloaded_bytes`, `total_bytes`) for HTTP sources. Bundled/magnet/
/// user-provided sources do not stream and never invoke `on_progress`.
pub async fn download_source_to_file_with_progress(
    source: &SourceUri,
    destination: &Path,
    on_progress: impl FnMut(u64, Option<u64>),
) -> Result<DownloadedFile, String> {
    download_source_to_file_with_progress_and_cancel(source, destination, on_progress, || false)
        .await
}

/// Like [`download_source_to_file_with_progress`], but lets callers abort a
/// streaming direct download without waiting for the process to exit.
pub async fn download_source_to_file_with_progress_and_cancel(
    source: &SourceUri,
    destination: &Path,
    on_progress: impl FnMut(u64, Option<u64>),
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<DownloadedFile, String> {
    match source {
        SourceUri::Http {
            url,
            sha256,
            size_bytes,
        } => {
            let expected_sha256 = if sha256.trim().is_empty() {
                None
            } else {
                Some(sha256.as_str())
            };
            download_http_streaming(
                url,
                destination,
                StreamOptions {
                    expected_sha256,
                    expected_size_bytes: *size_bytes,
                    max_bytes: None,
                    resume: true,
                },
                on_progress,
                is_cancelled,
            )
            .await
        }
        SourceUri::Bundled {
            path,
            sha256,
            size_bytes,
        } => {
            if is_cancelled() {
                return Err(DOWNLOAD_CANCELLED_MESSAGE.to_string());
            }
            copy_bundled_to_file(path, sha256, *size_bytes, destination).await
        }
        SourceUri::Magnet { uri, .. } => handle_torrent(uri).await,
        SourceUri::UserProvided { .. } => {
            Err("This source is user-provided and cannot be downloaded automatically.".to_string())
        }
    }
}

pub fn destination_for_source(
    root: &Path,
    platform: &str,
    subject_id: &str,
    source: &SourceUri,
    fallback_name: &str,
) -> PathBuf {
    let file_name = match source {
        SourceUri::Http { .. } => file_name_for_source(source, fallback_name),
        SourceUri::Bundled { .. } => file_name_for_source(source, fallback_name),
        SourceUri::Magnet { .. } => file_name_for_source(source, fallback_name),
        SourceUri::UserProvided { .. } => file_name_for_source(source, fallback_name),
    };
    root.join(safe_segment(platform))
        .join(safe_segment(subject_id))
        .join(file_name)
}

pub fn file_name_for_source(source: &SourceUri, fallback_name: &str) -> String {
    match source {
        SourceUri::Http { url, .. } => {
            file_name_from_url(url).unwrap_or_else(|| safe_segment(fallback_name))
        }
        SourceUri::Bundled { path, .. } => {
            file_name_from_path(path).unwrap_or_else(|| safe_segment(fallback_name))
        }
        SourceUri::Magnet { .. } => safe_segment(fallback_name),
        SourceUri::UserProvided { .. } => safe_segment(fallback_name),
    }
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Streams an HTTP body to `destination` without buffering the whole file in
/// memory. Writes to a sibling `<name>.part` file and atomically renames it
/// into place only after size/hash verification succeeds, so a crash never
/// leaves a partial file that looks complete.
pub async fn download_http_streaming(
    url: &str,
    destination: &Path,
    options: StreamOptions<'_>,
    mut on_progress: impl FnMut(u64, Option<u64>),
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<DownloadedFile, String> {
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Failed to create download folder: {error}"))?;
    }
    let part_path = part_path_for(destination);
    if is_cancelled() {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(DOWNLOAD_CANCELLED_MESSAGE.to_string());
    }

    // Decide whether we can resume an existing partial file. If anything looks
    // off we fall back to a clean download instead of risking a corrupt resume.
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut resume_offset: u64 = 0;
    if options.resume {
        if let Ok(metadata) = tokio::fs::metadata(&part_path).await {
            if metadata.is_file() && metadata.len() > 0 {
                match hash_existing_part(&part_path, hasher).await {
                    Ok((existing_hasher, existing_len)) => {
                        hasher = existing_hasher;
                        downloaded = existing_len;
                        resume_offset = existing_len;
                    }
                    Err(_) => {
                        let _ = tokio::fs::remove_file(&part_path).await;
                        hasher = Sha256::new();
                    }
                }
            }
        }
    } else {
        let _ = tokio::fs::remove_file(&part_path).await;
    }

    let client = reqwest::Client::builder()
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .user_agent(DOWNLOAD_USER_AGENT)
        .build()
        .map_err(|error| format!("Failed to initialize downloader: {error}"))?;
    let mut request = client.get(url);
    if resume_offset > 0 {
        request = request.header(RANGE, format!("bytes={resume_offset}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Failed to fetch source: {error}"))?;

    // If we asked for a range but the server replied 200 OK, it ignored the
    // Range header and is sending the whole file again: restart from scratch.
    let mut append = false;
    if resume_offset > 0 {
        if response.status() == StatusCode::PARTIAL_CONTENT {
            append = true;
        } else if response.status().is_success() {
            hasher = Sha256::new();
            downloaded = 0;
            let _ = tokio::fs::remove_file(&part_path).await;
        }
    }
    let mut response = response
        .error_for_status()
        .map_err(|error| format!("Source returned an error: {error}"))?;

    // For a 206 resume the body is only the remaining bytes, so add what we
    // already have on disk to report a meaningful total.
    let total_bytes = response
        .content_length()
        .map(|len| len + resume_offset)
        .or(options.expected_size_bytes);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&part_path)
        .await
        .map_err(|error| format!("Failed to open download file: {error}"))?;

    on_progress(downloaded, total_bytes);
    let mut last_report = Instant::now();

    loop {
        if is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(DOWNLOAD_CANCELLED_MESSAGE.to_string());
        }
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => return Err(format!("Failed to read source body: {error}")),
        };
        if is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(DOWNLOAD_CANCELLED_MESSAGE.to_string());
        }
        downloaded += chunk.len() as u64;
        if let Some(max_bytes) = options.max_bytes {
            if downloaded > max_bytes {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(format!(
                    "Download exceeded the size limit of {max_bytes} bytes."
                ));
            }
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("Failed to write download: {error}"))?;

        if last_report.elapsed() >= PROGRESS_REPORT_INTERVAL {
            on_progress(downloaded, total_bytes);
            last_report = Instant::now();
        }
    }

    on_progress(downloaded, total_bytes);

    file.flush()
        .await
        .map_err(|error| format!("Failed to flush download: {error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("Failed to persist download: {error}"))?;
    drop(file);

    if let Err(error) = validate_size(downloaded, options.expected_size_bytes) {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(error);
    }

    let actual_sha256 = hex::encode(hasher.finalize());
    if let Some(expected_sha256) = options.expected_sha256 {
        if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(format!(
                "SHA256 mismatch: expected {expected_sha256}, got {actual_sha256}"
            ));
        }
    }

    if tokio::fs::metadata(destination).await.is_ok() {
        let _ = tokio::fs::remove_file(destination).await;
    }
    tokio::fs::rename(&part_path, destination)
        .await
        .map_err(|error| format!("Failed to finalize download: {error}"))?;

    Ok(DownloadedFile {
        path: destination.to_path_buf(),
        sha256: actual_sha256,
    })
}

pub async fn remove_partial_download(destination: &Path) -> Result<(), String> {
    let part_path = part_path_for(destination);
    match tokio::fs::remove_file(&part_path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to remove partial download {}: {error}",
            part_path.display()
        )),
    }
}

fn part_path_for(destination: &Path) -> PathBuf {
    let mut file_name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    file_name.push(".part");
    destination.with_file_name(file_name)
}

/// Re-hashes an existing `.part` file off the async runtime so a resumed
/// download continues the SHA-256 from exactly where it left off.
async fn hash_existing_part(path: &Path, hasher: Sha256) -> Result<(Sha256, u64), String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(&path)
            .map_err(|error| format!("Failed to open partial download: {error}"))?;
        let mut hasher = hasher;
        let mut buffer = [0_u8; 64 * 1024];
        let mut total = 0_u64;
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("Failed to read partial download: {error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            total += read as u64;
        }
        Ok((hasher, total))
    })
    .await
    .map_err(|error| format!("Failed to inspect partial download: {error}"))?
}

async fn copy_bundled_to_file(
    path: &str,
    expected_sha256: &str,
    expected_size_bytes: Option<u64>,
    destination: &Path,
) -> Result<DownloadedFile, String> {
    let bytes =
        builtin_demo::asset_bytes(path).ok_or_else(|| format!("Unknown bundled asset: {path}"))?;
    let actual_sha256 = hex::encode(Sha256::digest(bytes));
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!(
            "SHA256 mismatch: expected {expected_sha256}, got {actual_sha256}"
        ));
    }
    validate_size(bytes.len() as u64, expected_size_bytes)?;

    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::fs::write(destination, bytes)
        .await
        .map_err(|error| error.to_string())?;

    Ok(DownloadedFile {
        path: destination.to_path_buf(),
        sha256: actual_sha256,
    })
}

fn validate_size(actual_size_bytes: u64, expected_size_bytes: Option<u64>) -> Result<(), String> {
    if let Some(expected_size_bytes) = expected_size_bytes {
        if actual_size_bytes != expected_size_bytes {
            return Err(format!(
                "Size mismatch: expected {expected_size_bytes} bytes, got {actual_size_bytes} bytes"
            ));
        }
    }

    Ok(())
}

async fn handle_torrent(_magnet: &str) -> Result<DownloadedFile, String> {
    Err("Torrent handler is not implemented in v1.".to_string())
}

fn file_name_from_url(input: &str) -> Option<String> {
    let parsed = Url::parse(input).ok()?;
    let segment = parsed
        .path_segments()?
        .rfind(|segment| !segment.is_empty())?;
    Some(safe_segment(segment))
}

fn file_name_from_path(input: &str) -> Option<String> {
    let segment = input.split('/').rfind(|segment| !segment.is_empty())?;
    Some(safe_segment(segment))
}

pub fn safe_segment(value: &str) -> String {
    let sanitized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(|ch| ch == '.' || ch == '-')
        .chars()
        .take(120)
        .collect::<String>();

    if sanitized.is_empty() {
        "item".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_safe_path_segments() {
        assert_eq!(safe_segment("../bad:name.exe"), "bad-name.exe");
        assert_eq!(safe_segment(""), "item");
    }

    #[test]
    fn part_path_is_sibling_with_part_suffix() {
        let part = part_path_for(Path::new("C:/games/nes/sonic.nes"));
        assert_eq!(part, Path::new("C:/games/nes/sonic.nes.part"));
    }

    #[test]
    fn direct_download_registry_cancels_and_forgets_token() {
        let registry = DirectDownloadRegistry::new();
        let guard = registry.start("repo::game").unwrap();
        let token = guard.token();

        assert!(!token.is_cancelled());
        assert!(registry.cancel("repo::game").unwrap());
        assert!(token.is_cancelled());

        drop(guard);
        assert!(!registry.cancel("repo::game").unwrap());
    }

    #[test]
    fn direct_download_speed_meter_reports_delta_per_second() {
        let mut meter = DirectDownloadSpeedMeter::new();
        let start = Instant::now();

        assert_eq!(meter.sample_at(1_000, start), 0);
        assert_eq!(
            meter.sample_at(3_000, start + Duration::from_millis(500)),
            4_000
        );
    }

    #[test]
    fn direct_download_speed_meter_ignores_invalid_samples() {
        let mut meter = DirectDownloadSpeedMeter::new();
        let start = Instant::now();

        assert_eq!(meter.sample_at(2_000, start), 0);
        assert_eq!(meter.sample_at(2_000, start + Duration::from_secs(1)), 0);
        assert_eq!(meter.sample_at(1_000, start + Duration::from_secs(2)), 0);
        assert_eq!(meter.sample_at(2_000, start + Duration::from_secs(2)), 0);
    }

    #[tokio::test]
    async fn cancelled_http_download_removes_partial_file_before_request() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("game.bin");
        let part = part_path_for(&destination);
        std::fs::write(&part, b"partial").unwrap();

        let error = download_http_streaming(
            "https://example.com/game.bin",
            &destination,
            StreamOptions {
                expected_sha256: None,
                expected_size_bytes: None,
                max_bytes: None,
                resume: true,
            },
            |_, _| {},
            || true,
        )
        .await
        .unwrap_err();

        assert_eq!(error, DOWNLOAD_CANCELLED_MESSAGE);
        assert!(!part.exists());
    }

    #[tokio::test]
    async fn resumed_hash_matches_full_file_digest() {
        let temp = tempfile::tempdir().unwrap();
        let part = temp.path().join("game.bin.part");
        std::fs::write(&part, b"fusion-").unwrap();

        // Continue the hash from the partial bytes, then feed the remainder.
        let (mut hasher, len) = hash_existing_part(&part, Sha256::new()).await.unwrap();
        assert_eq!(len, 7);
        hasher.update(b"launcher");
        let resumed = hex::encode(hasher.finalize());

        // Must equal hashing "fusion-launcher" in one shot.
        assert_eq!(resumed, hex::encode(Sha256::digest(b"fusion-launcher")));
        assert_eq!(
            resumed,
            "120b3930657e4166c13a21e3ae527f8338cfeaaedbba0aac0c9d7e15e52bff16"
        );
    }

    #[test]
    fn hashes_files_in_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.bin");
        std::fs::write(&path, b"fusion-launcher").unwrap();

        assert_eq!(
            hash_file(&path).unwrap(),
            "120b3930657e4166c13a21e3ae527f8338cfeaaedbba0aac0c9d7e15e52bff16"
        );
    }

    #[tokio::test]
    async fn copies_bundled_assets_with_hash_verification() {
        let repo = crate::builtin_demo::repository_schema().unwrap();
        crate::builtin_demo::verify_embedded_assets(&repo).unwrap();
        let source = repo.catalog[0].downloads[0].clone();
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("fusion-launcher-smoke.nes");

        let file = download_source_to_file(&source, &destination)
            .await
            .unwrap();

        assert!(file.path.exists());
        assert_eq!(
            file.sha256,
            "566722254227d93e49751a866cf51ff7728917c9f04e970130d24850dce0a7f4"
        );
    }
}
