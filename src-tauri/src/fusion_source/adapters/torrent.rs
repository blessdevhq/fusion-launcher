//! DB-free, supervised libtorrent artifact sessions. The private owner never
//! receives the cascade destination, so an aborted caller cannot publish late.

use std::fs::{self, File, OpenOptions};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::FutureExt;
use tokio::sync::oneshot;

use super::torrent_relay::WebSeedRelay;
use super::{
    Cancellation, TransferLimits, TransferProgress, TransferProgressSink, TransferReceipt,
};
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety::{self, DirectoryGuard, FileIdentity};
use crate::fusion_source::resolver::{TorrentAdapter, TorrentFile, TorrentLayout, TorrentRequest};
use crate::libtorrent_engine::LibtorrentEngine;
use crate::torrent_engine::{EngineArtifactLayout, TorrentEngine};

pub struct LibtorrentSidecarAdapter {
    engine: Arc<dyn TorrentEngine>,
    staging_root: PathBuf,
}

impl LibtorrentSidecarAdapter {
    pub async fn probe(app: tauri::AppHandle, library_root: PathBuf) -> Result<Self, FusionError> {
        let engine = Arc::new(LibtorrentEngine::new(app));
        engine.probe_artifact().await.map_err(engine_error)?;
        Ok(Self::with_engine(engine, library_root))
    }

    /// Injection remains useful for a low-level host bridge and lifecycle tests.
    /// Production callers must first complete the native artifact protocol probe.
    pub fn with_engine(engine: Arc<dyn TorrentEngine>, library_root: PathBuf) -> Self {
        Self {
            engine,
            staging_root: library_root.join(".fusion-source").join("torrent-staging"),
        }
    }

    async fn session(
        &self,
        magnet: &str,
        request: Option<&TorrentRequest>,
        limits: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<CompletedSession, FusionError> {
        limits.validate()?;
        cancel.check()?;
        expected_info_hash(magnet)?;
        if let Some(request) = request {
            validate_layout(magnet, &request.layout, limits)?;
        }
        let stage = OwnedStage::new(&self.staging_root)?;
        let engine = Arc::clone(&self.engine);
        let magnet = magnet.to_owned();
        let request = request.cloned();
        let limits = limits.clone();
        let external_cancel = cancel.clone();
        let private_cancel = Cancellation::default();
        let guard = CancelOnDrop(private_cancel.clone());
        let (sender, receiver) = oneshot::channel();
        // This owner survives caller/task cancellation. All its paths and process
        // references remain private until both process tree and relay have joined.
        tokio::spawn(async move {
            let mut stage = stage;
            // Before add starts, a writer may appear even if the add future
            // fails or panics. Only positive quiescence permits Drop cleanup.
            stage.preserve = true;
            let mut relay = None;
            let result = AssertUnwindSafe(async { tokio::select! {
                _ = external_cancel.cancelled() => Err(cancelled()),
                _ = private_cancel.cancelled() => Err(cancelled()),
                _ = tokio::time::sleep(limits.timeout) => Err(FusionError::new(ErrorCode::TransferLimitExceeded,"Torrent delivery deadline exceeded.")),
                result = run_session(&engine, &magnet, request.as_ref(), &limits, &mut stage, &mut relay, progress) => result,
            }}).catch_unwind().await.unwrap_or_else(|_| Err(FusionError::new(ErrorCode::ProcessSupervisionFailed,"Torrent session panicked; stopping private writers.")));
            // Includes an add that failed after creating an engine entry but did
            // not return its id. Never infer stop from handle disposal alone.
            let stopped = engine
                .stop_for_path(&stage.directory.to_string_lossy())
                .await;
            let relay_stopped = match relay {
                Some(relay) => relay.stop().await,
                None => Ok(()),
            };
            let result = match (stopped, relay_stopped, result) {
                (Err(error), _, _) => {
                    stage.preserve = true;
                    Err(FusionError::new(
                        ErrorCode::ProcessSupervisionFailed,
                        format!(
                            "Torrent stop is unconfirmed; private staging was preserved: {error}"
                        ),
                    ))
                }
                (_, Err(error), _) => {
                    stage.preserve = true;
                    Err(error)
                }
                (_, _, Err(error)) => {
                    stage.preserve = false;
                    Err(error)
                }
                (_, _, Ok(layout)) => {
                    stage.preserve = false;
                    Ok(CompletedSession { layout, stage })
                }
            };
            // A dropped receiver destroys completed staging only after writers
            // have stopped. Nothing in this task knows a publication destination.
            let _ = sender.send(result);
        });
        let result = receiver.await.map_err(|_| {
            FusionError::new(
                ErrorCode::ProcessSupervisionFailed,
                "Torrent owner disappeared.",
            )
        })?;
        drop(guard);
        result
    }
}

fn engine_error(message: String) -> FusionError {
    FusionError::new(ErrorCode::Network, message)
}
fn cancelled() -> FusionError {
    FusionError::new(ErrorCode::Cancelled, "Torrent delivery was cancelled.")
}

struct CancelOnDrop(Cancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct OwnedStage {
    directory: PathBuf,
    _parent: DirectoryGuard,
    directory_guard: Option<DirectoryGuard>,
    reserved: Option<(PathBuf, FileIdentity, Option<File>)>,
    preserve: bool,
}

impl OwnedStage {
    fn new(parent: &Path) -> Result<Self, FusionError> {
        let parent_guard = fs_safety::ensure_directory(parent)?;
        let directory = parent.join(fs_safety::unique_name("artifact"));
        fs::create_dir(&directory)
            .map_err(|error| fs_safety::io_error("Reserve torrent staging", &directory, error))?;
        Ok(Self {
            directory_guard: Some(fs_safety::guard(&directory)?),
            directory,
            _parent: parent_guard,
            reserved: None,
            preserve: false,
        })
    }

    fn reserve(&mut self, file: &TorrentFile) -> Result<(), FusionError> {
        let path = self.directory.join(&file.path);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Native reads/writes remain allowed, but replacement is denied.
            // DELETE authority is acquired only after process-tree stop.
            options.share_mode(3).custom_flags(0x00200000);
        }
        let retained = options
            .open(&path)
            .map_err(|error| fs_safety::io_error("Reserve torrent container", &path, error))?;
        let identity = fs_safety::identity_from_file(&retained)?;
        self.reserved = Some((path, identity, Some(retained)));
        Ok(())
    }

    fn publish(&mut self, destination: &Path, size: u64) -> Result<(), FusionError> {
        let (path, identity, retained) = self.reserved.as_mut().ok_or_else(|| {
            FusionError::new(
                ErrorCode::IntegrityViolation,
                "Torrent has no reserved container.",
            )
        })?;
        // Reopen under immutable publication sharing; compare the descriptor
        // identities across this transition to reject a swapped leaf.
        if let Some(file) = retained.as_ref() {
            file.sync_all()
                .map_err(|error| fs_safety::io_error("Flush torrent container", path, error))?;
        }
        drop(retained.take());
        #[cfg(windows)]
        let file = crate::install_ownership::open_publication_file(path)
            .map_err(|error| fs_safety::io_error("Retain completed torrent", path, error))?;
        #[cfg(not(windows))]
        let file = fs_safety::open_read(path)?;
        let metadata = file
            .metadata()
            .map_err(|error| fs_safety::io_error("Inspect torrent container", path, error))?;
        fs_safety::check_metadata(&metadata, path)?;
        if !metadata.is_file()
            || metadata.len() != size
            || fs_safety::identity_from_file(&file)? != *identity
            || fs_safety::link_count(&file)? != 1
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Torrent output changed identity or disagrees with metadata.",
            ));
        }
        let _destination_parent = fs_safety::guard(destination.parent().ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidPath, "Torrent destination has no parent.")
        })?)?;
        #[cfg(windows)]
        fs_safety::publish_file_no_replace(&file, destination)?;
        #[cfg(not(windows))]
        {
            fs::hard_link(path, destination).map_err(|error| {
                fs_safety::io_error("Publish torrent container", destination, error)
            })?;
            fs::remove_file(path).map_err(|error| {
                fs_safety::io_error("Unlink private torrent container", path, error)
            })?;
        }
        Ok(())
    }
}

impl Drop for OwnedStage {
    fn drop(&mut self) {
        if self.preserve {
            return;
        }
        if let Some((path, identity, file)) = self.reserved.take() {
            // Closing our retained reservation now is safe: the supervisor has
            // already established process-tree and relay quiescence.
            drop(file);
            if fs_safety::file_identity(&path).is_ok_and(|actual| actual == identity) {
                let _ = fs_safety::unlink_file(&path);
            }
        }
        drop(self.directory_guard.take());
        // Non-recursive removal cannot delete unexpected/unowned files.
        let _ = fs::remove_dir(&self.directory);
    }
}

struct CompletedSession {
    layout: TorrentLayout,
    stage: OwnedStage,
}

fn converted_layout(layout: EngineArtifactLayout) -> TorrentLayout {
    TorrentLayout {
        info_hash: layout.info_hash,
        files: layout
            .files
            .into_iter()
            .map(|file| TorrentFile {
                path: file.path,
                size: file.size,
            })
            .collect(),
    }
}

fn expected_info_hash(magnet: &str) -> Result<String, FusionError> {
    let url = url::Url::parse(magnet)
        .map_err(|_| FusionError::new(ErrorCode::InvalidUri, "Invalid torrent magnet."))?;
    if url.scheme() != "magnet" {
        return Err(FusionError::new(
            ErrorCode::InvalidUri,
            "A magnet is required.",
        ));
    }
    let hashes = url
        .query_pairs()
        .filter(|(key, _)| key == "xt")
        .filter_map(|(_, value)| value.strip_prefix("urn:btih:").map(str::to_string))
        .collect::<Vec<_>>();
    if hashes.len() != 1 {
        return Err(FusionError::new(
            ErrorCode::InvalidUri,
            "Exactly one v1 torrent identity is required.",
        ));
    }
    normalized_info_hash(&hashes[0])
        .ok_or_else(|| FusionError::new(ErrorCode::InvalidUri, "Invalid v1 torrent identity."))
}

pub(crate) fn normalized_info_hash(value: &str) -> Option<String> {
    if value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(value.to_ascii_lowercase());
    }
    if value.len() != 32 {
        return None;
    }
    let mut decoded = Vec::with_capacity(20);
    let mut bits = 0u32;
    let mut count = 0;
    for byte in value.bytes() {
        let n = match byte.to_ascii_uppercase() {
            b'A'..=b'Z' => byte.to_ascii_uppercase() - b'A',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | u32::from(n);
        count += 5;
        if count >= 8 {
            count -= 8;
            decoded.push((bits >> count) as u8);
        }
    }
    Some(hex::encode(decoded))
}

fn validate_layout(
    magnet: &str,
    layout: &TorrentLayout,
    limits: &TransferLimits,
) -> Result<(), FusionError> {
    if Some(expected_info_hash(magnet)?) != normalized_info_hash(&layout.info_hash)
        || layout.files.len() != 1
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Artifact requires matching single-file torrent metadata.",
        ));
    }
    let file = &layout.files[0];
    fs_safety::validate_relative_path(&file.path)?;
    if file.path.contains(['/', '\\']) || file.size > limits.max_bytes {
        return Err(FusionError::new(
            ErrorCode::TransferLimitExceeded,
            "Torrent layout exceeds artifact limits.",
        ));
    }
    Ok(())
}

async fn run_session(
    engine: &Arc<dyn TorrentEngine>,
    magnet: &str,
    request: Option<&TorrentRequest>,
    limits: &TransferLimits,
    stage: &mut OwnedStage,
    relay: &mut Option<WebSeedRelay>,
    progress: Option<Arc<dyn TransferProgressSink>>,
) -> Result<TorrentLayout, FusionError> {
    let web_seeds = if let Some(request) = request {
        let owned =
            WebSeedRelay::start(&request.web_seeds, request.layout.files[0].size, limits).await?;
        let seeds = owned.seeds.clone();
        *relay = Some(owned);
        seeds
    } else {
        Vec::new()
    };
    let id = engine
        .add_artifact(
            magnet.to_string(),
            stage.directory.to_string_lossy().into_owned(),
            web_seeds,
        )
        .await
        .map_err(engine_error)?;
    let mut approved = None;
    loop {
        let stats = engine
            .stats(id)
            .ok_or_else(|| engine_error("Torrent runtime disappeared.".into()))?;
        if let Some(error) = stats.error {
            return Err(engine_error(error));
        }
        if let Some(layout) = stats.artifact_layout {
            let layout = converted_layout(layout);
            validate_layout(magnet, &layout, limits)?;
            if stats.inventory.as_ref()
                != Some(
                    &layout
                        .files
                        .iter()
                        .map(|file| file.path.clone())
                        .collect::<Vec<_>>(),
                )
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Torrent file inventory disagrees with artifact metadata.",
                ));
            }
            if let Some(request) = request {
                if normalized_info_hash(&layout.info_hash)
                    != normalized_info_hash(&request.layout.info_hash)
                    || layout.files != request.layout.files
                {
                    return Err(FusionError::new(
                        ErrorCode::InvalidManifest,
                        "Torrent metadata changed between discovery and transfer.",
                    ));
                }
                if approved.is_none() {
                    stage.reserve(&layout.files[0])?;
                    engine.approve_inventory(id).await.map_err(engine_error)?;
                    approved = Some(layout.clone());
                }
                if stats.downloaded_bytes > layout.files[0].size
                    || stats.total_bytes > limits.max_bytes
                {
                    return Err(FusionError::new(
                        ErrorCode::TransferLimitExceeded,
                        "Torrent reported oversized transfer.",
                    ));
                }
                if let Some(sink) = &progress {
                    sink.emit(TransferProgress {
                        downloaded_bytes: stats.downloaded_bytes,
                        total_bytes: layout.files[0].size,
                        speed_bytes_per_second: stats.download_speed_bytes_per_sec,
                    })?;
                }
                if stats.finished && stats.downloaded_bytes == layout.files[0].size {
                    return Ok(layout);
                }
            } else {
                return Ok(layout);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[async_trait]
impl TorrentAdapter for LibtorrentSidecarAdapter {
    async fn metadata(
        &self,
        magnet: &str,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TorrentLayout, FusionError> {
        Ok(self
            .session(magnet, None, limits, cancel, None)
            .await?
            .layout)
    }

    async fn fetch(
        &self,
        request: &TorrentRequest,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.fetch_with_progress(request, destination, limits, cancel, None)
            .await
    }

    async fn fetch_with_progress(
        &self,
        request: &TorrentRequest,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<TransferReceipt, FusionError> {
        let mut completed = self
            .session(&request.magnet, Some(request), limits, cancel, progress)
            .await?;
        // Deliberately synchronous: there is no cancellation boundary between
        // checking caller authority and no-replace publication into its staging.
        cancel.check()?;
        let bytes = completed.layout.files[0].size;
        completed.stage.publish(destination, bytes)?;
        Ok(TransferReceipt {
            bytes,
            final_url: request.magnet.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::torrent_engine::{EngineArtifactFile, EngineTorrentStats};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    struct FakeEntry {
        path: PathBuf,
        stats: EngineTorrentStats,
    }
    #[derive(Default)]
    struct FakeEngine {
        entries: Mutex<HashMap<usize, FakeEntry>>,
        next: AtomicUsize,
        approved: AtomicUsize,
        stops: AtomicUsize,
        finish: AtomicBool,
        stop_blocked: AtomicBool,
        stop_release: tokio::sync::Notify,
        stop_failed: AtomicBool,
        add_failed: AtomicBool,
        add_panics: AtomicBool,
        mismatch: AtomicBool,
        crash: AtomicBool,
        seeds: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl TorrentEngine for FakeEngine {
        async fn add_magnet(&self, _magnet: String, _save: String) -> Result<usize, String> {
            panic!("legacy entrypoint called")
        }
        async fn add_artifact(
            &self,
            _magnet: String,
            save: String,
            seeds: Vec<String>,
        ) -> Result<usize, String> {
            *self.seeds.lock().unwrap() = seeds;
            let id = self.next.fetch_add(1, Ordering::SeqCst);
            let hash = if self.mismatch.load(Ordering::SeqCst) {
                "f".repeat(40)
            } else {
                HASH.into()
            };
            self.entries.lock().unwrap().insert(
                id,
                FakeEntry {
                    path: save.into(),
                    stats: EngineTorrentStats {
                        total_bytes: 7,
                        inventory: Some(vec!["payload.bin".into()]),
                        artifact_layout: Some(EngineArtifactLayout {
                            info_hash: hash,
                            files: vec![EngineArtifactFile {
                                path: "payload.bin".into(),
                                size: 7,
                            }],
                        }),
                        error: self
                            .crash
                            .load(Ordering::SeqCst)
                            .then(|| "injected crash".into()),
                        ..Default::default()
                    },
                },
            );
            assert!(
                !self.add_panics.load(Ordering::SeqCst),
                "injected post-spawn add panic"
            );
            if self.add_failed.load(Ordering::SeqCst) {
                Err("add failed after spawn".into())
            } else {
                Ok(id)
            }
        }
        async fn approve_inventory(&self, id: usize) -> Result<(), String> {
            self.approved.fetch_add(1, Ordering::SeqCst);
            let mut entries = self.entries.lock().unwrap();
            let entry = entries.get_mut(&id).unwrap();
            assert!(
                entry.path.join("payload.bin").is_file(),
                "parent must reserve before approval"
            );
            if self.finish.load(Ordering::SeqCst) {
                fs::write(entry.path.join("payload.bin"), b"payload").unwrap();
                entry.stats.downloaded_bytes = 7;
                entry.stats.finished = true;
            }
            Ok(())
        }
        async fn pause(&self, _id: usize) -> Result<(), String> {
            Ok(())
        }
        async fn resume(&self, _id: usize) -> Result<(), String> {
            Ok(())
        }
        async fn stop(&self, _id: usize) -> Result<(), String> {
            Ok(())
        }
        async fn stop_for_path(&self, path: &str) -> Result<(), String> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            while self.stop_blocked.load(Ordering::SeqCst) {
                let notified = self.stop_release.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.stop_blocked.load(Ordering::SeqCst) {
                    notified.await;
                }
            }
            if self.stop_failed.load(Ordering::SeqCst) {
                return Err("writers unconfirmed".into());
            }
            self.entries
                .lock()
                .unwrap()
                .retain(|_, entry| entry.path != Path::new(path));
            Ok(())
        }
        async fn delete(&self, _id: usize, _delete: bool) -> Result<(), String> {
            panic!("legacy cleanup called")
        }
        fn exists(&self, id: usize) -> bool {
            self.entries.lock().unwrap().contains_key(&id)
        }
        fn stats(&self, id: usize) -> Option<EngineTorrentStats> {
            self.entries
                .lock()
                .unwrap()
                .get(&id)
                .map(|entry| entry.stats.clone())
        }
    }

    fn request() -> TorrentRequest {
        TorrentRequest {
            magnet: format!("magnet:?xt=urn:btih:{HASH}"),
            layout: TorrentLayout {
                info_hash: HASH.into(),
                files: vec![TorrentFile {
                    path: "payload.bin".into(),
                    size: 7,
                }],
            },
            web_seeds: vec!["https://example.org/payload.bin".into()],
        }
    }
    fn limits() -> TransferLimits {
        TransferLimits {
            max_bytes: 10,
            timeout: Duration::from_secs(5),
        }
    }
    fn stage_count(root: &Path) -> usize {
        fs::read_dir(root.join(".fusion-source/torrent-staging"))
            .map(|entries| entries.count())
            .unwrap_or(0)
    }
    async fn until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn torrent_hash_encoding_and_unsafe_layout_are_checked() {
        assert_eq!(
            normalized_info_hash("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            Some("0".repeat(40))
        );
        assert_eq!(
            normalized_info_hash("77777777777777777777777777777777"),
            Some("f".repeat(40))
        );
        assert!(normalized_info_hash("11111111111111111111111111111111").is_none());
        for filename in ["../payload.bin", "folder/payload.bin", "NUL", "file:stream"] {
            let mut request = request();
            request.layout.files[0].path = filename.into();
            assert!(
                validate_layout(&request.magnet, &request.layout, &limits()).is_err(),
                "{filename}"
            );
        }
    }

    #[tokio::test]
    async fn metadata_uses_no_payload_approval_and_stops_before_return() {
        let root = tempfile::tempdir().unwrap();
        let engine = Arc::new(FakeEngine::default());
        let adapter = LibtorrentSidecarAdapter::with_engine(engine.clone(), root.path().into());
        let layout = adapter
            .metadata(&request().magnet, &limits(), &Cancellation::default())
            .await
            .unwrap();
        assert_eq!(layout.files[0].size, 7);
        assert_eq!(engine.approved.load(Ordering::SeqCst), 0);
        assert_eq!(engine.stops.load(Ordering::SeqCst), 1);
        assert_eq!(stage_count(root.path()), 0);
    }

    #[tokio::test]
    async fn completed_transfer_stops_and_publishes_without_replacing() {
        let root = tempfile::tempdir().unwrap();
        let engine = Arc::new(FakeEngine::default());
        engine.finish.store(true, Ordering::SeqCst);
        let adapter = LibtorrentSidecarAdapter::with_engine(engine.clone(), root.path().into());
        let destination = root.path().join("artifact.part");
        let receipt = adapter
            .fetch(
                &request(),
                &destination,
                &limits(),
                &Cancellation::default(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.bytes, 7);
        assert_eq!(fs::read(&destination).unwrap(), b"payload");
        assert_eq!(engine.stops.load(Ordering::SeqCst), 1);
        assert_eq!(stage_count(root.path()), 0);
        assert!(engine.seeds.lock().unwrap()[0].starts_with("http://127.0.0.1:"));
        let error = adapter
            .fetch(
                &request(),
                &destination,
                &limits(),
                &Cancellation::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Io);
        assert_eq!(fs::read(destination).unwrap(), b"payload");
        assert_eq!(stage_count(root.path()), 0);
    }

    #[tokio::test]
    async fn failed_stop_preserves_private_stage_and_never_publishes() {
        let root = tempfile::tempdir().unwrap();
        let engine = Arc::new(FakeEngine::default());
        engine.finish.store(true, Ordering::SeqCst);
        engine.stop_failed.store(true, Ordering::SeqCst);
        let adapter = LibtorrentSidecarAdapter::with_engine(engine, root.path().into());
        let destination = root.path().join("artifact.part");
        let error = adapter
            .fetch(
                &request(),
                &destination,
                &limits(),
                &Cancellation::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProcessSupervisionFailed);
        assert!(!destination.exists());
        assert_eq!(stage_count(root.path()), 1);
    }

    #[tokio::test]
    async fn task_abort_retains_private_stage_until_writer_stop_is_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let engine = Arc::new(FakeEngine::default());
        engine.stop_blocked.store(true, Ordering::SeqCst);
        let adapter = LibtorrentSidecarAdapter::with_engine(engine.clone(), root.path().into());
        let destination = root.path().join("artifact.part");
        let caller_destination = destination.clone();
        let task = tokio::spawn(async move {
            adapter
                .fetch(
                    &request(),
                    &caller_destination,
                    &limits(),
                    &Cancellation::default(),
                )
                .await
        });
        until(|| engine.approved.load(Ordering::SeqCst) > 0).await;
        task.abort();
        let _ = task.await;
        until(|| engine.stops.load(Ordering::SeqCst) > 0).await;
        assert!(!destination.exists());
        assert_eq!(stage_count(root.path()), 1);
        engine.stop_blocked.store(false, Ordering::SeqCst);
        engine.stop_release.notify_waiters();
        until(|| stage_count(root.path()) == 0).await;
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn cancellation_timeout_crash_and_failed_add_all_stop_the_private_path() {
        for mode in ["cancel", "timeout", "crash", "add", "panic", "metadata"] {
            let root = tempfile::tempdir().unwrap();
            let engine = Arc::new(FakeEngine::default());
            engine.crash.store(mode == "crash", Ordering::SeqCst);
            engine.add_failed.store(mode == "add", Ordering::SeqCst);
            engine.add_panics.store(mode == "panic", Ordering::SeqCst);
            engine.mismatch.store(mode == "metadata", Ordering::SeqCst);
            let adapter = LibtorrentSidecarAdapter::with_engine(engine.clone(), root.path().into());
            let cancel = Cancellation::default();
            let trigger = cancel.clone();
            let local_limits = if mode == "timeout" {
                TransferLimits {
                    timeout: Duration::from_millis(50),
                    ..limits()
                }
            } else {
                limits()
            };
            if mode == "cancel" {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    trigger.cancel();
                });
            }
            assert!(
                adapter
                    .fetch(
                        &request(),
                        &root.path().join("artifact.part"),
                        &local_limits,
                        &cancel
                    )
                    .await
                    .is_err(),
                "{mode}"
            );
            assert_eq!(engine.stops.load(Ordering::SeqCst), 1, "{mode}");
            assert_eq!(stage_count(root.path()), 0, "{mode}");
            if mode == "metadata" {
                assert_eq!(engine.approved.load(Ordering::SeqCst), 0);
            }
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires generated native single-file seeder fixture and rebuilt EXE"]
    async fn native_artifact_adapter_stops_and_publishes() {
        use sha2::Digest;
        let executable = std::env::var_os("FUSION_NATIVE_SIDECAR_EXE").expect("fixture executable");
        let magnet = std::env::var("FUSION_NATIVE_MAGNET").expect("fixture magnet");
        let expected = std::env::var("FUSION_NATIVE_SHA256").expect("fixture digest");
        let root = tempfile::tempdir().unwrap();
        let engine = Arc::new(LibtorrentEngine::with_test_executable(executable.into()));
        engine.probe_artifact().await.unwrap();
        let adapter = LibtorrentSidecarAdapter::with_engine(engine, root.path().into());
        let limits = TransferLimits {
            max_bytes: 64 * 1024 * 1024,
            timeout: Duration::from_secs(90),
        };
        let cancel = Cancellation::default();
        let layout = adapter.metadata(&magnet, &limits, &cancel).await.unwrap();
        let request = TorrentRequest {
            magnet,
            layout,
            web_seeds: Vec::new(),
        };
        let destination = root.path().join("container.bin");
        adapter
            .fetch(&request, &destination, &limits, &cancel)
            .await
            .unwrap();
        assert_eq!(
            hex::encode(sha2::Sha256::digest(fs::read(&destination).unwrap())),
            expected
        );
        assert_eq!(stage_count(root.path()), 0);
    }
}
