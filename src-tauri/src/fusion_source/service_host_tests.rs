//! Real service/SQLite/cascade/CAS wiring with deterministic host bridges.
use super::*;
use crate::fusion_source::adapters::{PixeldrainUriResolver, TransferReceipt};
use crate::fusion_source::resolver::{TorrentFile, TorrentLayout, TorrentRequest};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use async_trait::async_trait;
use serde_json::json;
use std::fs;

const MAGNET: &str = "magnet:?xt=urn:btih:1111111111111111111111111111111111111111";
const PAYLOAD: &[u8] = b"fixture";

#[derive(Default)]
struct Events(Mutex<Vec<OperationProgress>>);
impl ProgressSink for Events {
    fn emit(&self, event: &OperationProgress) {
        self.0.lock().unwrap().push(event.clone());
    }
}

struct Documents(BTreeMap<String, Vec<u8>>);
#[async_trait]
impl ArtifactTransport for Documents {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        _: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        cancel.check()?;
        let bytes = self
            .0
            .get(url)
            .ok_or_else(|| FusionError::new(ErrorCode::Network, "Unexpected fixture URL."))?;
        fs::write(destination, bytes).unwrap();
        Ok(TransferReceipt {
            bytes: bytes.len() as u64,
            final_url: url.to_owned(),
        })
    }
}

#[derive(Default)]
struct Torrent {
    requests: Mutex<Vec<TorrentRequest>>,
    sink: Mutex<Option<Arc<dyn TransferProgressSink>>>,
}
#[async_trait]
impl TorrentAdapter for Torrent {
    async fn metadata(
        &self,
        _: &str,
        _: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TorrentLayout, FusionError> {
        cancel.check()?;
        Ok(TorrentLayout {
            info_hash: "1".repeat(40),
            files: vec![TorrentFile {
                path: "payload.bin".into(),
                size: PAYLOAD.len() as u64,
            }],
        })
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
        _: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<TransferReceipt, FusionError> {
        cancel.check()?;
        self.requests.lock().unwrap().push(request.clone());
        if let Some(sink) = &progress {
            sink.emit(TransferProgress {
                downloaded_bytes: PAYLOAD.len() as u64,
                total_bytes: PAYLOAD.len() as u64,
                speed_bytes_per_second: 7,
            })?;
        }
        *self.sink.lock().unwrap() = progress;
        fs::write(destination, PAYLOAD).unwrap();
        Ok(TransferReceipt {
            bytes: PAYLOAD.len() as u64,
            final_url: request.magnet.clone(),
        })
    }
}

async fn fixture(
    adapters: FusionHostAdapters,
) -> (
    tempfile::TempDir,
    FusionService,
    Arc<Events>,
    QualifiedGameId,
) {
    let temp = tempfile::tempdir().unwrap();
    let manifest = serde_json::to_vec(&json!({
        "schema_version":"1.0", "id":"game", "title":"Fixture", "version":"1",
        "layers":[{"id":"base", "kind":"base", "source":{"kind":"remote", "archive":"raw", "mirrors":[MAGNET,"https://cdn.example/payload.bin"]},
            "transport_digest":{"algorithm":"blake3","value":blake3::hash(PAYLOAD).to_hex().to_string()}, "size_bytes":PAYLOAD.len(),
            "critical_files":[{"path":"data.exe","role":"main_executable","blake3":blake3::hash(PAYLOAD).to_hex().to_string()}]}],
        "launch":{"id":"main","executable":"data.exe","role":"primary"}
    })).unwrap();
    let index = serde_json::to_vec(&json!({
        "schema_version":"1.0", "source_info":{"id":"source","name":"Fixture"},
        "games":[{"id":"game","title":"Fixture","version":"1","manifest_url":"game.json","manifest_blake3":blake3::hash(&manifest).to_hex().to_string()}]
    })).unwrap();
    let transport = Arc::new(Documents(BTreeMap::from([
        ("https://source.example/index.json".into(), index),
        ("https://source.example/game.json".into(), manifest),
    ])));
    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
        RepositoryStore::open(&temp.path().join("test.db")).unwrap(),
    ))));
    let events = Arc::new(Events::default());
    let service = FusionService::open(
        &temp.path().join("library"),
        storage,
        transport,
        events.clone(),
        ServiceEnvironment {
            appdata: temp.path().join("appdata"),
            local_appdata: temp.path().join("local"),
            display_width: 1920,
            display_height: 1080,
            refresh_rate: 60,
        },
    )
    .unwrap()
    .with_host_adapters(adapters)
    .unwrap();
    service
        .source_add("https://source.example/index.json")
        .await
        .unwrap();
    (
        temp,
        service,
        events,
        QualifiedGameId {
            source_id: "source".into(),
            game_id: "game".into(),
        },
    )
}

#[tokio::test]
async fn configured_adapters_deliver_torrent_with_webseeds_and_bound_progress() {
    let torrent = Arc::new(Torrent::default());
    let (_temp, service, events, game) = fixture(FusionHostAdapters {
        torrent: Some(torrent.clone()),
        cloud: vec![Arc::new(PixeldrainUriResolver)],
        ..Default::default()
    })
    .await;
    let caps = service.configured_capabilities();
    assert!(caps.contains("resolver.magnet.v1"));
    assert!(caps.contains("resolver.pixeldrain.v1"));
    assert!(!caps.contains("resolver.gofile.v1"));
    assert!(!caps.contains("patch.xdelta3.v1"));
    let result = service
        .game_install(
            &game,
            InstallRequest {
                raw_paths: BTreeMap::from([("base".into(), "data.exe".into())]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    assert!(result.transfer.is_none());
    assert_eq!(
        torrent.requests.lock().unwrap()[0].web_seeds,
        ["https://cdn.example/payload.bin"]
    );
    let captured = events.0.lock().unwrap().clone();
    assert!(captured.iter().any(|event| event.transfer.is_some()
        && event.layer_id.as_deref() == Some("base")
        && event.operation_id == result.operation_id));
    assert!(captured
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    let count = captured.len();
    let late = torrent
        .sink
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .emit(TransferProgress {
            downloaded_bytes: 7,
            total_bytes: 7,
            speed_bytes_per_second: 0,
        });
    assert_eq!(late.unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(events.0.lock().unwrap().len(), count);
    // Release the test bridge's retained callback cycle, like a completed native session.
    torrent.sink.lock().unwrap().take();
}

#[tokio::test]
async fn baseline_service_does_not_advertise_unconfigured_extensions() {
    let (_temp, service, _, _) = fixture(FusionHostAdapters::default()).await;
    for capability in [
        "resolver.magnet.v1",
        "resolver.pixeldrain.v1",
        "patch.xdelta3.v1",
        "patch.bsdiff.v1",
    ] {
        assert!(!service.configured_capabilities().contains(capability));
    }
}

#[tokio::test]
async fn host_capability_requires_the_physically_verified_and_trusted_patch_binary() {
    use sha2::Digest;
    let tools = tempfile::tempdir().unwrap();
    let executable = tools.path().join("xdelta3.exe");
    fs::write(&executable, b"approved executable fixture").unwrap();
    let digest = hex::encode(sha2::Sha256::digest(b"approved executable fixture"));
    for mode in ["wrong_hash", "untrusted", "missing", "verified"] {
        fs::write(tools.path().join("fusion-source-host.json"), json!({
            "schema_version":1,"patch_tools":[{
                "format":"xdelta3","executable":if mode == "missing" {tools.path().join("absent.exe")} else {executable.clone()},
                "sha256":if mode == "wrong_hash" {"0".repeat(64)} else {digest.clone()},
                "version":"fixture-v1","trusted":mode != "untrusted"
            }]
        }).to_string()).unwrap();
        let configured =
            crate::fusion_source::host_tools::load_host_patch_tools(tools.path()).unwrap();
        let (_temp, service, _, _) = fixture(FusionHostAdapters {
            patch_tools: configured.registry,
            ..Default::default()
        })
        .await;
        assert_eq!(
            service
                .configured_capabilities()
                .contains("patch.xdelta3.v1"),
            mode == "verified",
            "{mode}"
        );
        assert!(!service
            .configured_capabilities()
            .contains("patch.bsdiff.v1"));
    }
}

#[tokio::test]
async fn cancelled_and_superseded_operations_reject_transfer_events() {
    let (_temp, service, events, game) = fixture(FusionHostAdapters::default()).await;
    let (owner, _) = service.begin_operation(&game, "install").unwrap();
    let sink = OperationTransferSink {
        service: service.clone(),
        game: game.clone(),
        operation_id: owner.id.clone(),
        layer_id: "base".into(),
    };
    service.game_cancel(&game).unwrap();
    let progress = TransferProgress {
        downloaded_bytes: 1,
        total_bytes: 7,
        speed_bytes_per_second: 1,
    };
    assert_eq!(
        sink.emit(progress.clone()).unwrap_err().code,
        ErrorCode::Cancelled
    );
    let cancelled = owner
        .finish(Err(FusionError::new(
            ErrorCode::Cancelled,
            "Fixture cancelled.",
        )))
        .unwrap_err();
    assert_eq!(cancelled.code, ErrorCode::Cancelled);
    let count = events.0.lock().unwrap().len();
    let (next, _) = service.begin_operation(&game, "install").unwrap();
    assert_eq!(sink.emit(progress).unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(events.0.lock().unwrap().len(), count + 1);
    next.finish(Ok(())).unwrap();
}
