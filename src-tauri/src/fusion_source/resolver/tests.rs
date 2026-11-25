use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use super::*;
use crate::fusion_source::adapters::{
    ArtifactTransport, Cancellation, SecureHttpsTransport, TransferLimits, TransferReceipt,
};
use crate::fusion_source::canonical::QualifiedGameId;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::plan::{ProviderCriticalFile, ProviderLockRequirement};
use crate::fusion_source::schema::*;

enum Outcome {
    Bytes(Vec<u8>),
    Error(ErrorCode),
    Cancel,
}

#[derive(Default)]
struct MockTransport {
    outcomes: Mutex<VecDeque<Outcome>>,
    calls: Mutex<Vec<String>>,
}

impl MockTransport {
    fn new(outcomes: impl IntoIterator<Item = Outcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ArtifactTransport for MockTransport {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        _: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.calls.lock().unwrap().push(url.into());
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected mock fetch");
        match outcome {
            Outcome::Bytes(bytes) => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(destination)
                    .unwrap();
                file.write_all(&bytes).unwrap();
                Ok(TransferReceipt {
                    bytes: bytes.len() as u64,
                    final_url: url.into(),
                })
            }
            Outcome::Error(code) => {
                Err(FusionError::new(code, "mock HTTP 429/503 or network error"))
            }
            Outcome::Cancel => {
                cancel.cancel();
                Err(FusionError::new(ErrorCode::Cancelled, "cancelled"))
            }
        }
    }
}

fn digest(bytes: &[u8]) -> DigestSpec {
    DigestSpec {
        algorithm: DigestAlgorithm::Blake3,
        value: blake3::hash(bytes).to_hex().to_string(),
    }
}
fn limits() -> TransferLimits {
    TransferLimits {
        max_bytes: 1024 * 1024,
        timeout: Duration::from_secs(2),
    }
}
fn mirrors(urls: &[&str]) -> Vec<String> {
    urls.iter().map(|url| url.to_string()).collect()
}

#[tokio::test]
async fn sequential_mirrors_fall_back_on_network_status_and_digest_failures() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::new([
        Outcome::Error(ErrorCode::Network),
        Outcome::Bytes(b"corrupt".to_vec()),
        Outcome::Bytes(b"correct".to_vec()),
    ]));
    let cascade = MirrorCascadeResolver::new(transport.clone());
    let output = temp.path().join("payload");
    let receipt = cascade
        .acquire(
            &mirrors(&[
                "https://one.example/a",
                "https://two.example/b",
                "https://three.example/c",
            ]),
            &digest(b"correct"),
            &output,
            &limits(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.mirror_index, 2);
    assert_eq!(transport.calls().len(), 3);
    assert_eq!(fs::read(output).unwrap(), b"correct");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn cancelled_attempt_is_terminal_and_never_contacts_next_mirror() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::new([
        Outcome::Cancel,
        Outcome::Bytes(b"correct".to_vec()),
    ]));
    let error = MirrorCascadeResolver::new(transport.clone())
        .acquire(
            &mirrors(&["https://one.example/a", "https://two.example/b"]),
            &digest(b"correct"),
            &temp.path().join("payload"),
            &limits(),
            &Cancellation::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn digest_rejection_and_size_limits_do_not_publish_or_replace_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::new([
        Outcome::Bytes(b"wrong".to_vec()),
        Outcome::Bytes(vec![0; 20]),
    ]));
    let resolver = MirrorCascadeResolver::new(transport.clone());
    let output = temp.path().join("payload");
    let one = mirrors(&["https://example.org/a"]);
    assert_eq!(
        resolver
            .acquire(
                &one,
                &digest(b"correct"),
                &output,
                &limits(),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::TransportDigestMismatch
    );
    assert!(!output.exists());
    let limited = TransferLimits {
        max_bytes: 5,
        ..limits()
    };
    assert_eq!(
        resolver
            .acquire(
                &one,
                &digest(&[0; 20]),
                &output,
                &limited,
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::TransferLimitExceeded
    );
    fs::write(&output, b"owned existing").unwrap();
    assert_eq!(
        resolver
            .acquire(
                &one,
                &digest(b"correct"),
                &output,
                &limits(),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::SnapshotConflict
    );
    assert_eq!(fs::read(output).unwrap(), b"owned existing");
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn cancellation_notification_and_secure_http_policy_are_fail_closed() {
    let cancellation = Cancellation::default();
    let other = cancellation.clone();
    let waiter = tokio::spawn(async move {
        other.cancelled().await;
    });
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    cancellation.cancelled().await;
    let temp = tempfile::tempdir().unwrap();
    for url in [
        "http://example.org/a",
        "https://127.0.0.1/a",
        "https://user:token@example.org/a",
        "file:///etc/passwd",
    ] {
        assert_eq!(
            SecureHttpsTransport
                .fetch(
                    url,
                    &temp.path().join("file"),
                    &limits(),
                    &Cancellation::default()
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidUri
        );
    }
    assert!(!temp.path().join("file").exists());
}

const MAGNET: &str = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";

struct MockTorrent {
    requests: Mutex<Vec<TorrentRequest>>,
    fail: bool,
    layout: TorrentLayout,
}

impl MockTorrent {
    fn new(fail: bool) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            fail,
            layout: TorrentLayout {
                info_hash: "0123456789012345678901234567890123456789".into(),
                files: vec![TorrentFile {
                    path: "payload.zip".into(),
                    size: 7,
                }],
            },
        }
    }
}

#[async_trait]
impl TorrentAdapter for MockTorrent {
    async fn metadata(
        &self,
        _: &str,
        _: &TransferLimits,
        _: &Cancellation,
    ) -> Result<TorrentLayout, FusionError> {
        Ok(self.layout.clone())
    }
    async fn fetch(
        &self,
        request: &TorrentRequest,
        destination: &Path,
        _: &TransferLimits,
        _: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.requests.lock().unwrap().push(request.clone());
        if self.fail {
            return Err(FusionError::new(ErrorCode::Network, "torrent failed"));
        }
        fs::write(destination, b"correct").unwrap();
        Ok(TransferReceipt {
            bytes: 7,
            final_url: request.magnet.clone(),
        })
    }
}

#[tokio::test]
async fn bep19_seeds_require_matching_torrent_filename_and_are_explicitly_passed() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::default());
    let torrent = Arc::new(MockTorrent::new(false));
    let resolver = MirrorCascadeResolver::new(transport.clone()).with_torrent(torrent.clone());
    let receipt = resolver
        .acquire(
            &mirrors(&[
                MAGNET,
                "https://cdn.example/payload.zip?signature=abc",
                "https://cdn.example/download",
            ]),
            &digest(b"correct"),
            &temp.path().join("payload"),
            &limits(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        receipt.web_seeds,
        ["https://cdn.example/payload.zip?signature=abc"]
    );
    assert_eq!(
        torrent.requests.lock().unwrap()[0].web_seeds,
        receipt.web_seeds
    );
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn incompatible_http_seed_remains_a_sequential_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::new([Outcome::Bytes(b"correct".to_vec())]));
    let torrent = Arc::new(MockTorrent::new(true));
    let receipt = MirrorCascadeResolver::new(transport.clone())
        .with_torrent(torrent.clone())
        .acquire(
            &mirrors(&[MAGNET, "https://cdn.example/download"]),
            &digest(b"correct"),
            &temp.path().join("payload"),
            &limits(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.mirror_index, 1);
    assert!(torrent.requests.lock().unwrap()[0].web_seeds.is_empty());
    assert_eq!(transport.calls(), ["https://cdn.example/download"]);
}

#[tokio::test]
async fn unverified_torrent_metadata_cannot_authorize_writes() {
    let temp = tempfile::tempdir().unwrap();
    let mut torrent = MockTorrent::new(false);
    torrent.layout.info_hash = "f".repeat(40);
    let torrent = Arc::new(torrent);
    let resolver = MirrorCascadeResolver::new(Arc::new(MockTransport::default()))
        .with_torrent(torrent.clone());
    assert_eq!(
        resolver
            .acquire(
                &mirrors(&[MAGNET]),
                &digest(b"correct"),
                &temp.path().join("payload"),
                &limits(),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidManifest
    );
    assert!(torrent.requests.lock().unwrap().is_empty());
    let seeded = format!("{MAGNET}&ws=http%3A%2F%2Flocalhost%2Fpayload.zip");
    assert_eq!(
        resolver
            .acquire(
                &[seeded],
                &digest(b"correct"),
                &temp.path().join("payload"),
                &limits(),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidUri
    );
}

struct Cloud {
    direct: String,
}
#[async_trait]
impl CloudUriResolver for Cloud {
    fn schemes(&self) -> Vec<&str> {
        vec!["pixeldrain"]
    }
    fn supports_scheme(&self, scheme: &str) -> bool {
        scheme == "pixeldrain"
    }
    async fn resolve(&self, _: &str, _: &Cancellation) -> Result<String, FusionError> {
        Ok(self.direct.clone())
    }
}

#[tokio::test]
async fn cloud_plugins_only_delegate_validated_https_and_remain_optional() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::new([Outcome::Bytes(b"correct".to_vec())]));
    let resolver = MirrorCascadeResolver::new(transport.clone()).with_cloud(Arc::new(Cloud {
        direct: "https://cdn.example/signed?id=abc".into(),
    }));
    resolver
        .acquire(
            &mirrors(&["pixeldrain://owned-id"]),
            &digest(b"correct"),
            &temp.path().join("payload"),
            &limits(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(transport.calls(), ["https://cdn.example/signed?id=abc"]);
    let bad = MirrorCascadeResolver::new(transport.clone()).with_cloud(Arc::new(Cloud {
        direct: "http://localhost/secret".into(),
    }));
    assert_eq!(
        bad.acquire(
            &mirrors(&["pixeldrain://owned-id"]),
            &digest(b"correct"),
            &temp.path().join("bad"),
            &limits(),
            &Cancellation::default()
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidUri
    );
    let missing = MirrorCascadeResolver::new(transport.clone());
    assert_eq!(
        missing
            .acquire(
                &mirrors(&["gofile://owned-id"]),
                &digest(b"correct"),
                &temp.path().join("missing"),
                &limits(),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::MissingCapability
    );
    assert_eq!(transport.calls().len(), 1);
}

#[tokio::test]
async fn native_pixeldrain_resolver_uses_secure_transport_and_advertises_only_its_scheme() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::new([Outcome::Bytes(b"correct".to_vec())]));
    let resolver = MirrorCascadeResolver::new(transport.clone()).with_cloud(Arc::new(
        crate::fusion_source::adapters::PixeldrainUriResolver,
    ));
    assert_eq!(
        resolver.configured_capabilities(),
        BTreeSet::from(["resolver.https.v1".into(), "resolver.pixeldrain.v1".into()])
    );
    resolver
        .acquire(
            &mirrors(&["pixeldrain://AbCd1234"]),
            &digest(b"correct"),
            &temp.path().join("payload"),
            &limits(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        transport.calls(),
        ["https://pixeldrain.com/api/file/AbCd1234"]
    );
}

#[tokio::test]
async fn base32_magnet_matches_hex_metadata_and_bad_digest_is_not_published() {
    let temp = tempfile::tempdir().unwrap();
    let mut torrent = MockTorrent::new(false);
    torrent.layout.info_hash = "0".repeat(40);
    let resolver = MirrorCascadeResolver::new(Arc::new(MockTransport::default()))
        .with_torrent(Arc::new(torrent));
    let magnet = format!("magnet:?xt=urn:btih:{}", "A".repeat(32));
    let output = temp.path().join("payload");
    assert_eq!(
        resolver
            .acquire(
                std::slice::from_ref(&magnet),
                &digest(b"tampered"),
                &output,
                &limits(),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::TransportDigestMismatch
    );
    assert!(!output.exists());
    resolver
        .acquire(
            &[magnet],
            &digest(b"correct"),
            &output,
            &limits(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(fs::read(output).unwrap(), b"correct");
}

#[tokio::test]
async fn unconfigured_modular_scheme_never_reaches_a_transport() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::default());
    let resolver = MirrorCascadeResolver::new(transport.clone());
    assert_eq!(
        resolver
            .acquire(
                &mirrors(&["future-provider://owned-id"]),
                &digest(b"correct"),
                &temp.path().join("payload"),
                &limits(),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::MissingCapability
    );
    assert!(transport.calls().is_empty());
}

struct ProgressTransport(Arc<MockTransport>);
#[async_trait]
impl ArtifactTransport for ProgressTransport {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.0.fetch(url, destination, limits, cancel).await
    }
    async fn fetch_with_progress(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn crate::fusion_source::adapters::TransferProgressSink>>,
    ) -> Result<TransferReceipt, FusionError> {
        let receipt = self.fetch(url, destination, limits, cancel).await?;
        if let Some(sink) = progress {
            sink.emit(crate::fusion_source::adapters::TransferProgress {
                downloaded_bytes: receipt.bytes,
                total_bytes: receipt.bytes,
                speed_bytes_per_second: 7,
            })?;
        }
        Ok(receipt)
    }
}
#[derive(Default)]
struct TransferEvents(Mutex<Vec<crate::fusion_source::adapters::TransferProgress>>);
impl crate::fusion_source::adapters::TransferProgressSink for TransferEvents {
    fn emit(
        &self,
        progress: crate::fusion_source::adapters::TransferProgress,
    ) -> Result<(), FusionError> {
        self.0.lock().unwrap().push(progress);
        Ok(())
    }
}

#[tokio::test]
async fn transfer_progress_reaches_https_and_cloud_cascade_downloads() {
    let temp = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::new([
        Outcome::Bytes(b"correct".to_vec()),
        Outcome::Bytes(b"correct".to_vec()),
    ]));
    let resolver = MirrorCascadeResolver::new(Arc::new(ProgressTransport(transport.clone())))
        .with_cloud(Arc::new(
            crate::fusion_source::adapters::PixeldrainUriResolver,
        ));
    let events = Arc::new(TransferEvents::default());
    for (index, uri) in ["https://cdn.example/payload", "pixeldrain://AbCd1234"]
        .iter()
        .enumerate()
    {
        resolver
            .acquire_with_progress(
                &[uri.to_string()],
                &digest(b"correct"),
                &temp.path().join(format!("payload-{index}")),
                &limits(),
                &Cancellation::default(),
                Some(events.clone()),
            )
            .await
            .unwrap();
    }
    assert_eq!(events.0.lock().unwrap().len(), 2);
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|event| event.downloaded_bytes == 7));
    assert_eq!(
        transport.calls(),
        [
            "https://cdn.example/payload",
            "https://pixeldrain.com/api/file/AbCd1234"
        ]
    );
}

fn manifest_bytes() -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version":"1.0","id":"game","title":"Fixture","version":"1",
      "layers":[{"id":"base","kind":"base","source":{"kind":"user_provided"},"critical_files":[{"path":"main.exe","blake3":digest(b"exe").value,"role":"main_executable"}]}],
      "launch":{"id":"main","executable":"main.exe","role":"primary"}})).unwrap()
}
fn entry(bytes: &[u8]) -> SourceGameEntry {
    SourceGameEntry {
        id: "game".into(),
        title: "Fixture".into(),
        version: "1".into(),
        platform: None,
        cover_url: None,
        manifest_url: "https://source.example/manifests/game.json".into(),
        manifest_blake3: digest(bytes).value,
    }
}
fn index_bytes(hash: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version":"1.0","source_info":{"id":"source","name":"Fixture"},"games":[{"id":"game","title":"Fixture","version":"1","manifest_url":"https://source.example/manifests/game.json","manifest_blake3":hash}]})).unwrap()
}

#[tokio::test]
async fn raw_manifest_hash_precedes_upcast_and_same_hash_reuses_cache() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = manifest_bytes();
    let transport = Arc::new(MockTransport::new([
        Outcome::Bytes(bytes.clone()),
        Outcome::Bytes(b"invalid json".to_vec()),
    ]));
    let fetcher = ManifestFetcher::open(temp.path(), transport.clone()).unwrap();
    let first = fetcher
        .fetch_manifest(
            "source",
            &entry(&bytes),
            &BTreeMap::new(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    let second = fetcher
        .fetch_manifest(
            "source",
            &entry(&bytes),
            &BTreeMap::new(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(!first.cache_reused && second.cache_reused);
    assert_eq!(first.raw_bytes, bytes);
    assert_eq!(first.canonical, second.canonical);
    assert_eq!(transport.calls().len(), 1);
    let mut bad = entry(&bytes);
    bad.manifest_blake3 = "0".repeat(64);
    assert_eq!(
        fetcher
            .fetch_manifest("source", &bad, &BTreeMap::new(), &Cancellation::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::ManifestDigestMismatch
    );
    assert_eq!(transport.calls().len(), 2);
    assert!(!temp
        .path()
        .join(".fusion-source/manifests")
        .join(format!("{}.json", bad.manifest_blake3))
        .exists());
}

#[tokio::test]
async fn corrupt_manifest_cache_is_repaired_only_with_expected_fetched_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = manifest_bytes();
    let transport = Arc::new(MockTransport::new([
        Outcome::Bytes(bytes.clone()),
        Outcome::Bytes(bytes.clone()),
    ]));
    let fetcher = ManifestFetcher::open(temp.path(), transport.clone()).unwrap();
    fetcher
        .fetch_manifest(
            "source",
            &entry(&bytes),
            &BTreeMap::new(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    let cache = temp
        .path()
        .join(".fusion-source/manifests")
        .join(format!("{}.json", digest(&bytes).value));
    fs::write(&cache, b"corrupt cache").unwrap();
    let repaired = fetcher
        .fetch_manifest(
            "source",
            &entry(&bytes),
            &BTreeMap::new(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(!repaired.cache_reused);
    assert_eq!(fs::read(cache).unwrap(), bytes);
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn strict_sync_checks_current_index_each_time_and_binds_source_identity() {
    let temp = tempfile::tempdir().unwrap();
    let hash = digest(&manifest_bytes()).value;
    let transport = Arc::new(MockTransport::new([
        Outcome::Bytes(index_bytes(&hash)),
        Outcome::Bytes(index_bytes(&"f".repeat(64))),
    ]));
    let fetcher = ManifestFetcher::open(temp.path(), transport.clone()).unwrap();
    let game = QualifiedGameId {
        source_id: "source".into(),
        game_id: "game".into(),
    };
    let proof = fetcher
        .confirm_current(
            "https://source.example/source-index.json",
            &game,
            &hash,
            &Cancellation::default(),
        )
        .await
        .unwrap();
    proof
        .validate(&game, &hash, Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        proof
            .validate(
                &QualifiedGameId {
                    source_id: "other".into(),
                    ..game.clone()
                },
                &hash,
                Duration::from_secs(2)
            )
            .unwrap_err()
            .code,
        ErrorCode::StrictSyncRequired
    );
    assert_eq!(
        fetcher
            .confirm_current(
                "https://source.example/source-index.json",
                &game,
                &hash,
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::StrictSyncRequired
    );
    assert_eq!(transport.calls().len(), 2);
}

struct ReleaseClient {
    release: ReleaseMetadata,
    calls: Mutex<usize>,
}
#[async_trait]
impl GithubReleaseClient for ReleaseClient {
    async fn release(
        &self,
        _: &GithubReleaseProvider,
        _: &Cancellation,
    ) -> Result<ReleaseMetadata, FusionError> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.release.clone())
    }
}

fn provider() -> GithubReleaseProvider {
    GithubReleaseProvider {
        repo: "owner/runtime".into(),
        asset_mask: "runtime-*.zip".into(),
        channel: ReleaseChannel::Latest,
        tag: None,
    }
}
fn attestation(version: &str, bytes: &[u8]) -> DynamicAssetAttestation {
    DynamicAssetAttestation {
        release_id: format!("release-{version}"),
        tag: version.into(),
        asset_id: format!("asset-{version}"),
        asset_url: format!(
            "https://github.com/owner/runtime/releases/download/{version}/runtime-win.zip"
        ),
        transport_digest: digest(bytes),
        critical_files: vec![CriticalFileSpec {
            path: "main.exe".into(),
            blake3: digest(bytes).value,
            role: CriticalFileRole::MainExecutable,
        }],
    }
}
fn release(expected: &DynamicAssetAttestation) -> ReleaseMetadata {
    ReleaseMetadata {
        id: expected.release_id.clone(),
        tag: expected.tag.clone(),
        draft: false,
        prerelease: true,
        assets: vec![ReleaseAsset {
            id: expected.asset_id.clone(),
            name: "runtime-win.zip".into(),
            url: expected.asset_url.clone(),
            api_digest: Some(expected.transport_digest.clone()),
        }],
    }
}
fn requirement() -> ProviderLockRequirement {
    ProviderLockRequirement {
        release_and_asset: true,
        transport_digest: true,
        critical_files: vec![ProviderCriticalFile {
            path: "main.exe".into(),
            role: CriticalFileRole::MainExecutable,
        }],
    }
}

#[tokio::test]
async fn dynamic_provider_requires_authoritative_expected_hashes_and_pins_exact_identity() {
    let expected = attestation("nightly-123", b"runtime");
    let client = Arc::new(ReleaseClient {
        release: release(&expected),
        calls: Mutex::new(0),
    });
    let resolver = DynamicProviderResolver::new(client.clone());
    assert_eq!(
        resolver
            .resolve(&provider(), &requirement(), None, &Cancellation::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::ProviderResolutionFailed
    );
    assert_eq!(*client.calls.lock().unwrap(), 0);
    let lock = resolver
        .resolve(
            &provider(),
            &requirement(),
            Some(&expected),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    let stored = lock.snapshot_lock();
    assert_eq!(
        stored.asset_url.as_deref(),
        Some(expected.asset_url.as_str())
    );
    assert_eq!(stored.release_tag.as_deref(), Some("nightly-123"));
    assert_eq!(stored.transport_digest, expected.transport_digest);
    assert_eq!(stored.critical_files, expected.critical_files);
    let mut wrong = expected.clone();
    wrong.asset_id = "other-asset".into();
    assert_eq!(
        resolver
            .resolve(
                &provider(),
                &requirement(),
                Some(&wrong),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::ProviderResolutionFailed
    );
    let mut untrusted = expected;
    untrusted.critical_files.clear();
    assert_eq!(
        resolver
            .resolve(
                &provider(),
                &requirement(),
                Some(&untrusted),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::ProviderResolutionFailed
    );
}

#[tokio::test]
async fn github_client_resolves_nightly_api_metadata_with_bounded_secure_transport() {
    let temp = tempfile::tempdir().unwrap();
    let body=serde_json::to_vec(&json!(
        {"id":3,"tag_name":"nightly-3","prerelease":true,"assets":[{"id":4,"name":"runtime-win.zip","browser_download_url":"https://github.com/owner/runtime/releases/download/nightly-3/runtime-win.zip","digest":format!("sha256:{}","a".repeat(64))}]}
    )).unwrap();
    let transport = Arc::new(MockTransport::new([Outcome::Bytes(body)]));
    let client = SecureGithubReleaseClient::open(temp.path(), transport.clone()).unwrap();
    let nightly = GithubReleaseProvider {
        channel: ReleaseChannel::Nightly,
        tag: Some("nightly-3".into()),
        ..provider()
    };
    let found = client
        .release(&nightly, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(found.id, "3");
    assert_eq!(found.tag, "nightly-3");
    assert_eq!(
        found.assets[0].api_digest.as_ref().unwrap().algorithm,
        DigestAlgorithm::Sha256
    );
    assert_eq!(
        transport.calls(),
        ["https://api.github.com/repos/owner/runtime/releases/tags/nightly-3"]
    );
}

struct Inventory {
    roots: Vec<PathBuf>,
}
#[async_trait]
impl StorefrontInventory for Inventory {
    async fn candidates(
        &self,
        _: LocalStore,
        _: Option<&str>,
        _: &Cancellation,
    ) -> Result<Vec<PathBuf>, FusionError> {
        Ok(self.roots.clone())
    }
}

#[tokio::test]
async fn local_locator_verifies_keys_and_reports_volume_without_mutating_originals() {
    let temp = tempfile::tempdir().unwrap();
    let bad = temp.path().join("bad");
    let good = temp.path().join("good");
    let library = temp.path().join("library");
    for path in [&bad, &good, &library] {
        fs::create_dir(path).unwrap();
    }
    fs::write(bad.join("main.exe"), b"old").unwrap();
    fs::write(good.join("main.exe"), b"correct").unwrap();
    let readonly = fs::metadata(good.join("main.exe"))
        .unwrap()
        .permissions()
        .readonly();
    let resolver = LocalLocatorResolver::new().with_inventory(Arc::new(Inventory {
        roots: vec![good.clone()],
    }));
    let locator = LocalLocatorSpec {
        store: LocalStore::Steam,
        app_id: Some("123".into()),
        path_hints: vec![bad.to_string_lossy().into()],
        key_files: vec![CriticalFileSpec {
            path: "main.exe".into(),
            blake3: digest(b"correct").value,
            role: CriticalFileRole::MainExecutable,
        }],
    };
    let found = resolver
        .resolve(&locator, &library, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(found.root, good);
    assert_eq!(found.volume_id, found.library_volume_id);
    assert_eq!(
        fs::metadata(good.join("main.exe"))
            .unwrap()
            .permissions()
            .readonly(),
        readonly
    );
    fs::write(good.join("main.exe"), b"updated by storefront").unwrap();
    assert_eq!(
        resolver
            .resolve(&locator, &library, &Cancellation::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::LocalBaseModified
    );
    assert_eq!(fs::read(bad.join("main.exe")).unwrap(), b"old");
}

#[tokio::test]
async fn dynamic_lock_survives_workspace_update_and_rollback_without_re_resolving_latest() {
    use crate::fusion_source::capabilities::CapabilityRegistry;
    use crate::fusion_source::cas::CasStore;
    use crate::fusion_source::config_templates::TemplateContext;
    use crate::fusion_source::evolution::SchemaUpcaster;
    use crate::fusion_source::layers::LayerInput;
    use crate::fusion_source::persistence::PersistenceRoots;
    use crate::fusion_source::plan::{PlanBuilder, PlanOptions};
    use crate::fusion_source::workspace::{WorkspaceAssembler, WorkspaceBuildOptions};
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("library");
    let appdata = temp.path().join("appdata");
    let local = temp.path().join("localappdata");
    for path in [&library, &appdata, &local] {
        fs::create_dir(path).unwrap();
    }
    let canonical=SchemaUpcaster::upcast_game_manifest("source",&json!({"schema_version":"1.0","id":"game","title":"Dynamic fixture","version":"1",
      "layers":[{"id":"runtime","kind":"runtime","mount_path":"runtime","source":{"kind":"remote","mirrors":["https://example.org/pinned.zip"],"archive":"zip"},"transport_digest":digest(b"pinned"),
      "critical_files":[{"path":"main.exe","blake3":digest(b"pinned").value,"role":"main_executable"}],"dynamic_provider":{"repo":"owner/runtime","asset_mask":"runtime-*.zip","channel":"latest"}}],
      "runtime":{"layer_id":"runtime","executable":"runtime/main.exe","default_channel":"dynamic"}}).to_string(),&BTreeMap::new()).unwrap();
    let registry = CapabilityRegistry::current_client()
        .with("resolver.https.v1")
        .with("provider.github_releases.v1")
        .with("proc.supervisor.v1");
    let plan = PlanBuilder::build(&canonical, &PlanOptions::default(), &registry).unwrap();
    let cas = CasStore::open(&library).unwrap();
    let assembler = WorkspaceAssembler::open(
        &library,
        &QualifiedGameId {
            source_id: "source".into(),
            game_id: "game".into(),
        },
    )
    .unwrap();
    let context = TemplateContext {
        workspace_dir: library.clone(),
        appdata_dir: appdata.clone(),
        local_appdata_dir: local.clone(),
        display_width: 1280,
        display_height: 720,
        refresh_rate: 60,
        enabled_addons: BTreeSet::new(),
    };
    let roots = PersistenceRoots {
        appdata,
        local_appdata: local,
    };
    let mut original = None;
    for (index, bytes) in [b"nightly-one".as_slice(), b"nightly-two".as_slice()]
        .into_iter()
        .enumerate()
    {
        let expected = attestation(&format!("nightly-{index}"), bytes);
        let client = Arc::new(ReleaseClient {
            release: release(&expected),
            calls: Mutex::new(0),
        });
        let resolved = DynamicProviderResolver::new(client.clone())
            .resolve(
                &provider(),
                &requirement(),
                Some(&expected),
                &Cancellation::default(),
            )
            .await
            .unwrap();
        let runtime = temp.path().join(format!("runtime-{index}"));
        fs::create_dir(&runtime).unwrap();
        fs::write(runtime.join("main.exe"), bytes).unwrap();
        let object = cas.import_directory(&runtime).unwrap();
        let lock = resolved.snapshot_lock();
        let inputs = BTreeMap::from([(
            "runtime".into(),
            LayerInput::Cas {
                digest: object.digest,
                runtime_lock: Some(lock.clone()),
            },
        )]);
        let workspace = assembler
            .assemble(
                &canonical,
                &plan,
                &inputs,
                &context,
                &roots,
                &WorkspaceBuildOptions::default(),
            )
            .unwrap();
        if index == 0 {
            original = Some((workspace.snapshot.id.clone(), lock));
        }
        drop(workspace);
        assert_eq!(*client.calls.lock().unwrap(), 1);
    }
    let rollback = assembler.rollback().unwrap();
    let (id, lock) = original.unwrap();
    assert_eq!(rollback.snapshot.id, id);
    match &rollback.snapshot.inputs["runtime"] {
        LayerInput::Cas {
            runtime_lock: Some(restored),
            ..
        } => assert_eq!(restored, &lock),
        other => panic!("lost dynamic lock: {other:?}"),
    }
    assert_eq!(
        fs::read(rollback.snapshot.workspace_root.join("runtime/main.exe")).unwrap(),
        b"nightly-one"
    );
}
