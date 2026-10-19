use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::fusion_source::adapters::{
    https_url, validate_cloud_uri, ArtifactTransport, Cancellation, TransferLimits,
    TransferProgressSink, TransferReceipt,
};
use crate::fusion_source::cas::validate_hash;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use crate::fusion_source::schema::{DigestAlgorithm, DigestSpec};

/// Owned staging follows the transfer into its blocking verification worker, so
/// aborting an async caller cannot leave a writer or verification task publishing
/// into a directory whose ownership guard has already been released.
pub(super) struct StagedFile {
    pub(super) path: PathBuf,
    directory: PathBuf,
    _parent: fs_safety::DirectoryGuard,
    directory_guard: Option<fs_safety::DirectoryGuard>,
}

impl StagedFile {
    pub(super) fn new(parent: &Path) -> Result<Self, FusionError> {
        let parent_guard = fs_safety::guard(parent)?;
        let directory = parent.join(fs_safety::unique_name(".delivery"));
        fs::create_dir(&directory).map_err(|error| {
            fs_safety::io_error("Reserve private delivery directory", &directory, error)
        })?;
        Ok(Self {
            path: directory.join("artifact.part"),
            directory_guard: Some(fs_safety::guard(&directory)?),
            directory,
            _parent: parent_guard,
        })
    }

    pub(super) fn cleanup(&mut self) -> Result<(), FusionError> {
        if fs::symlink_metadata(&self.path).is_ok() {
            fs_safety::unlink_file(&self.path)?;
        }
        drop(self.directory_guard.take());
        if self.directory.exists() {
            fs::remove_dir(&self.directory).map_err(|error| {
                fs_safety::io_error("Remove empty delivery directory", &self.directory, error)
            })?;
        }
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

struct CancelOnDrop(Cancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[async_trait]
pub trait CloudUriResolver: Send + Sync {
    /// Enumerate implemented schemes so capability discovery grows with providers.
    /// Legacy plugins may continue resolving their known schemes without advertising them.
    fn schemes(&self) -> Vec<&str> {
        Vec::new()
    }
    fn supports_scheme(&self, scheme: &str) -> bool;
    /// Resolves only a locator, never downloads a payload. The resulting URL is
    /// revalidated by the cascade and the normal secure HTTP transport.
    async fn resolve(&self, uri: &str, cancel: &Cancellation) -> Result<String, FusionError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorrentFile {
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorrentLayout {
    /// The v1 info hash from validated metadata, in the magnet's encoding.
    pub info_hash: String,
    pub files: Vec<TorrentFile>,
}

#[derive(Clone, Debug)]
pub struct TorrentRequest {
    pub magnet: String,
    pub layout: TorrentLayout,
    /// BEP 19 seeds, admitted only after matching single-file torrent structure.
    pub web_seeds: Vec<String>,
}

/// Integration point for the low-level sidecar engine, rather than the legacy
/// game installation commands. Metadata must be checked before permitting writes;
/// fetch must stop and await every writer before returning or being dropped.
#[async_trait]
pub trait TorrentAdapter: Send + Sync {
    async fn metadata(
        &self,
        magnet: &str,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TorrentLayout, FusionError>;

    /// Produces the single declared container as an exclusive staging file.
    /// Multi-file torrents need an explicit container adapter; they are not
    /// silently converted into archives with an unrelated transport digest.
    async fn fetch(
        &self,
        request: &TorrentRequest,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError>;

    async fn fetch_with_progress(
        &self,
        request: &TorrentRequest,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        _progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<TransferReceipt, FusionError> {
        self.fetch(request, destination, limits, cancel).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub bytes: u64,
    pub mirror_index: usize,
    pub resolved_url: String,
    pub digest: DigestSpec,
    pub web_seeds: Vec<String>,
}

pub struct MirrorCascadeResolver {
    https: Arc<dyn ArtifactTransport>,
    torrent: Option<Arc<dyn TorrentAdapter>>,
    cloud: Vec<Arc<dyn CloudUriResolver>>,
}

impl MirrorCascadeResolver {
    pub fn new(https: Arc<dyn ArtifactTransport>) -> Self {
        Self {
            https,
            torrent: None,
            cloud: Vec::new(),
        }
    }

    pub fn with_torrent(mut self, torrent: Arc<dyn TorrentAdapter>) -> Self {
        self.torrent = Some(torrent);
        self
    }

    pub fn with_cloud(mut self, resolver: Arc<dyn CloudUriResolver>) -> Self {
        self.cloud.push(resolver);
        self
    }

    /// Merge these into the host's registry. Merely compiling TorrentAdapter or
    /// CloudUriResolver does not claim an implementation is available.
    pub fn configured_capabilities(&self) -> BTreeSet<String> {
        let mut capabilities = BTreeSet::from(["resolver.https.v1".into()]);
        if self.torrent.is_some() {
            capabilities.insert("resolver.magnet.v1".into());
        }
        for plugin in &self.cloud {
            for scheme in plugin.schemes() {
                if plugin.supports_scheme(scheme)
                    && validate_cloud_uri(&format!("{scheme}://identifier")).is_ok()
                {
                    capabilities.insert(format!("resolver.{scheme}.v1"));
                }
            }
        }
        capabilities
    }

    pub async fn acquire(
        &self,
        mirrors: &[String],
        digest: &DigestSpec,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<DeliveryReceipt, FusionError> {
        self.acquire_with_progress(mirrors, digest, destination, limits, cancel, None)
            .await
    }

    pub async fn acquire_with_progress(
        &self,
        mirrors: &[String],
        digest: &DigestSpec,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<DeliveryReceipt, FusionError> {
        limits.validate()?;
        validate_hash(&digest.value)?;
        if mirrors.is_empty() {
            return Err(FusionError::new(
                ErrorCode::InvalidUri,
                "A layer needs a mirror.",
            ));
        }
        for mirror in mirrors {
            validate_mirror(mirror)?;
        }
        cancel.check()?;
        let parent = destination.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "A delivery destination needs a parent.",
            )
        })?;
        let _parent = fs_safety::guard(parent)?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(FusionError::new(
                ErrorCode::SnapshotConflict,
                "Delivery cannot replace an existing file.",
            ));
        }
        let mut failures = Vec::new();
        let mut last_error = None;
        for (index, mirror) in mirrors.iter().enumerate() {
            cancel.check()?;
            let mut temporary = StagedFile::new(parent)?;
            let attempt = self
                .fetch_one(
                    mirror,
                    mirrors,
                    &temporary.path,
                    limits,
                    cancel,
                    progress.clone(),
                )
                .await;
            let result = match attempt {
                Ok((receipt, seeds)) => {
                    let destination = destination.to_owned();
                    let expected = digest.clone();
                    let cancellation = cancel.clone();
                    let max_bytes = limits.max_bytes;
                    let count = receipt.bytes;
                    let abort = Cancellation::default();
                    let abort_guard = CancelOnDrop(abort.clone());
                    let verification = tokio::task::spawn_blocking(move || {
                        let result = publish_verified(
                            &temporary.path,
                            &destination,
                            &expected,
                            max_bytes,
                            count,
                            &cancellation,
                            &abort,
                        );
                        temporary.cleanup()?;
                        result
                    })
                    .await
                    .map_err(|_| {
                        FusionError::new(ErrorCode::Io, "The verification worker failed.")
                    })?;
                    drop(abort_guard);
                    verification.map(|_| DeliveryReceipt {
                        bytes: receipt.bytes,
                        mirror_index: index,
                        resolved_url: receipt.final_url,
                        digest: digest.clone(),
                        web_seeds: seeds,
                    })
                }
                Err(error) => {
                    // Failure to delete means writers did not quiesce or ownership
                    // was lost. Never continue another mirror into that state.
                    temporary.cleanup()?;
                    Err(error)
                }
            };
            match result {
                Ok(receipt) => return Ok(receipt),
                Err(error)
                    if matches!(
                        error.code,
                        ErrorCode::Cancelled
                            | ErrorCode::Io
                            | ErrorCode::ProcessSupervisionFailed
                            | ErrorCode::InvalidPath
                            | ErrorCode::SnapshotConflict
                    ) =>
                {
                    return Err(error)
                }
                Err(error) => {
                    failures.push(serde_json::json!({"mirrorIndex":index,"code":error.code}));
                    last_error = Some(error);
                }
            }
        }
        Err(last_error
            .unwrap_or_else(|| {
                FusionError::new(ErrorCode::MirrorsExhausted, "No mirror succeeded.")
            })
            .with_detail("attempts", failures))
    }

    async fn fetch_one(
        &self,
        mirror: &str,
        all_mirrors: &[String],
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<(TransferReceipt, Vec<String>), FusionError> {
        let uri = url::Url::parse(mirror)
            .map_err(|_| FusionError::new(ErrorCode::InvalidUri, "Invalid mirror URI."))?;
        match uri.scheme() {
            "https" => {
                let receipt = self
                    .https
                    .fetch_with_progress(mirror, destination, limits, cancel, progress)
                    .await?;
                https_url(&receipt.final_url)?;
                Ok((receipt, Vec::new()))
            }
            "magnet" => {
                let adapter = self.torrent.as_ref().ok_or_else(|| {
                    FusionError::new(
                        ErrorCode::MissingCapability,
                        "No low-level torrent adapter is configured.",
                    )
                })?;
                let (magnet, native_seeds) = sanitized_magnet(mirror)?;
                let layout = adapter.metadata(&magnet, limits, cancel).await?;
                validate_layout(&magnet, &layout, limits)?;
                let mut candidates = all_mirrors.to_vec();
                candidates.extend(native_seeds);
                let seeds = compatible_web_seeds(&layout, &candidates)?;
                let request = TorrentRequest {
                    magnet,
                    layout,
                    web_seeds: seeds.clone(),
                };
                let receipt = adapter
                    .fetch_with_progress(&request, destination, limits, cancel, progress)
                    .await?;
                if receipt.bytes != request.layout.files[0].size {
                    return Err(FusionError::new(
                        ErrorCode::TransferLimitExceeded,
                        "The torrent container size differs from its verified metadata.",
                    ));
                }
                Ok((receipt, seeds))
            }
            scheme => {
                let resolver = self
                    .cloud
                    .iter()
                    .find(|resolver| resolver.supports_scheme(scheme))
                    .ok_or_else(|| {
                        FusionError::new(
                            ErrorCode::MissingCapability,
                            "No cloud URI plugin is configured.",
                        )
                        .with_detail("scheme", scheme)
                    })?;
                let direct = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(FusionError::new(ErrorCode::Cancelled,"Cloud resolution was cancelled.")),
                    result = tokio::time::timeout(limits.timeout,resolver.resolve(mirror,cancel)) => result
                        .map_err(|_| FusionError::new(ErrorCode::Network,"Cloud URI resolution timed out."))??,
                };
                https_url(&direct)?;
                let receipt = self
                    .https
                    .fetch_with_progress(&direct, destination, limits, cancel, progress)
                    .await?;
                https_url(&receipt.final_url)?;
                Ok((receipt, Vec::new()))
            }
        }
    }
}

fn validate_mirror(value: &str) -> Result<(), FusionError> {
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(FusionError::new(
            ErrorCode::InvalidUri,
            "Mirror URI contains whitespace or control characters.",
        ));
    }
    let url = url::Url::parse(value)
        .map_err(|_| FusionError::new(ErrorCode::InvalidUri, "Invalid mirror URI."))?;
    match url.scheme() {
        "https" => {
            https_url(value)?;
        }
        "magnet" => {
            magnet_hash(value)?;
            for (key, value) in url.query_pairs() {
                if matches!(key.as_ref(), "ws" | "xs") {
                    https_url(&value)?;
                }
            }
        }
        _ => {
            validate_cloud_uri(value)?;
        }
    }
    Ok(())
}

fn magnet_hash(value: &str) -> Result<String, FusionError> {
    let url = url::Url::parse(value)
        .map_err(|_| FusionError::new(ErrorCode::InvalidUri, "Invalid magnet."))?;
    let hash = url
        .query_pairs()
        .filter(|(key, _)| key == "xt")
        .find_map(|(_, value)| value.strip_prefix("urn:btih:").map(str::to_string))
        .ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidUri, "A v1 torrent info hash is required.")
        })?;
    if !value.starts_with("magnet:?")
        || !((hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            || (hash.len() == 32
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_alphabetic() || (b'2'..=b'7').contains(&byte))))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidUri,
            "Invalid torrent info hash.",
        ));
    }
    normalized_btih(&hash)
}

/// Both magnet encodings denote the same 20-byte v1 info hash.
fn normalized_btih(value: &str) -> Result<String, FusionError> {
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(value.to_ascii_lowercase());
    }
    if value.len() != 32 {
        return Err(FusionError::new(
            ErrorCode::InvalidUri,
            "Invalid torrent info hash.",
        ));
    }
    let mut bytes = Vec::with_capacity(20);
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    for character in value.bytes() {
        let digit = match character.to_ascii_uppercase() {
            b'A'..=b'Z' => character.to_ascii_uppercase() - b'A',
            b'2'..=b'7' => character - b'2' + 26,
            _ => {
                return Err(FusionError::new(
                    ErrorCode::InvalidUri,
                    "Invalid base32 info hash.",
                ))
            }
        };
        accumulator = (accumulator << 5) | u32::from(digit);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((accumulator >> bits) as u8);
            accumulator &= (1u32 << bits) - 1;
        }
    }
    Ok(hex::encode(bytes))
}

fn sanitized_magnet(value: &str) -> Result<(String, Vec<String>), FusionError> {
    let mut url = url::Url::parse(value)
        .map_err(|_| FusionError::new(ErrorCode::InvalidUri, "Invalid magnet."))?;
    let pairs = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    let mut seeds = Vec::new();
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            match key.as_str() {
                "ws" => {
                    https_url(&value)?;
                    seeds.push(value);
                }
                // Exact metadata-source URLs are not native delivery URLs. A
                // future adapter may fetch them through the secure HTTP client.
                "xs" => {
                    https_url(&value)?;
                }
                _ => {
                    query.append_pair(&key, &value);
                }
            }
        }
    }
    Ok((url.to_string(), seeds))
}

fn validate_layout(
    magnet: &str,
    layout: &TorrentLayout,
    limits: &TransferLimits,
) -> Result<(), FusionError> {
    if magnet_hash(magnet)? != normalized_btih(&layout.info_hash)? || layout.files.len() != 1 {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "A container transfer requires matching single-file torrent metadata.",
        ));
    }
    let file = &layout.files[0];
    fs_safety::validate_relative_path(&file.path)?;
    if file.path.contains(['/', '\\']) || file.size > limits.max_bytes {
        return Err(FusionError::new(
            ErrorCode::TransferLimitExceeded,
            "Torrent container layout is incompatible or exceeds its limit.",
        ));
    }
    Ok(())
}

fn compatible_web_seeds(
    layout: &TorrentLayout,
    mirrors: &[String],
) -> Result<Vec<String>, FusionError> {
    let filename = &layout.files[0].path;
    let mut encoding = url::Url::parse("https://example.org/").expect("constant URL");
    encoding.set_path(filename);
    let encoded = encoding.path();
    let mut seeds = Vec::new();
    for mirror in mirrors {
        if mirror.starts_with("https:") {
            let url = https_url(mirror)?;
            // A generic /download endpoint or a directory seed cannot prove a
            // direct container matches the torrent file's URL layout.
            if url.path().rsplit('/').next() == encoded.rsplit('/').next()
                && !seeds.contains(mirror)
            {
                seeds.push(mirror.clone());
            }
        }
    }
    Ok(seeds)
}

fn publish_verified(
    source: &Path,
    destination: &Path,
    expected: &DigestSpec,
    max_bytes: u64,
    count: u64,
    cancel: &Cancellation,
    abort: &Cancellation,
) -> Result<(), FusionError> {
    use sha2::Digest;
    cancel.check()?;
    abort.check()?;
    #[cfg(windows)]
    let mut file = crate::install_ownership::open_publication_file(source)
        .map_err(|error| fs_safety::io_error("Retain completed transfer", source, error))?;
    #[cfg(not(windows))]
    let mut file = fs_safety::open_read(source)?;
    let metadata = file
        .metadata()
        .map_err(|error| fs_safety::io_error("Inspect completed transfer", source, error))?;
    fs_safety::check_metadata(&metadata, source)?;
    if !metadata.is_file() || metadata.len() > max_bytes || metadata.len() != count {
        return Err(FusionError::new(
            ErrorCode::TransferLimitExceeded,
            "A transport returned inconsistent or oversized output.",
        ));
    }
    let mut blake3 = blake3::Hasher::new();
    let mut sha256 = sha2::Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        cancel.check()?;
        abort.check()?;
        let size = file
            .read(&mut buffer)
            .map_err(|error| fs_safety::io_error("Verify completed transfer", source, error))?;
        if size == 0 {
            break;
        }
        match expected.algorithm {
            DigestAlgorithm::Blake3 => {
                blake3.update(&buffer[..size]);
            }
            DigestAlgorithm::Sha256 => sha256.update(&buffer[..size]),
        }
    }
    let actual = match expected.algorithm {
        DigestAlgorithm::Blake3 => blake3.finalize().to_hex().to_string(),
        DigestAlgorithm::Sha256 => hex::encode(sha256.finalize()),
    };
    if !actual.eq_ignore_ascii_case(&expected.value) {
        return Err(FusionError::new(
            ErrorCode::TransportDigestMismatch,
            "The downloaded container failed its expected digest.",
        ));
    }
    cancel.check()?;
    abort.check()?;
    #[cfg(windows)]
    fs_safety::publish_file_no_replace(&file, destination)?;
    #[cfg(not(windows))]
    {
        fs::hard_link(source, destination).map_err(|error| {
            fs_safety::io_error("Publish completed transfer", destination, error)
        })?;
        fs::remove_file(source)
            .map_err(|error| fs_safety::io_error("Remove transfer staging", source, error))?;
    }
    Ok(())
}
