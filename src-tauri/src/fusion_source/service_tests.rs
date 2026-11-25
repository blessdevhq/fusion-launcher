//! Facade tests cross the real SQLite, resolver, CAS and workspace boundaries.
//! Payloads stay local and no commercial content, provider or emulator is used.

use super::adapters::{ArtifactTransport, Cancellation, TransferLimits, TransferReceipt};
use super::canonical::QualifiedGameId;
use super::dto::{GameOptions, InstallRequest, OperationProgress};
use super::errors::{ErrorCode, FusionError};
use super::service::{FusionService, FusionStorage, ProgressSink, ServiceEnvironment};
use super::telemetry::{CrashOutcome, CrashReportBuilder, TelemetrySanitizer};
use super::workspace::WorkspaceSnapshot;
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

const INDEX: &str = "https://example.com/source/index.json";
const MANIFEST: &str = "https://example.com/source/game.json";
const RUNTIME_ONE: &[u8] = b"fixture runtime version one";
const RUNTIME_TWO: &[u8] = b"fixture runtime version two";

#[tokio::test]
async fn manual_backup_copies_current_detached_save_offline_without_switching_version() {
    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let original = active(&f);
    let save = original.workspace_root.join("saves/progress.dat");
    fs::remove_file(&save).unwrap();
    fs::write(&save, b"latest detached save").unwrap();
    f.transport.calls.lock().unwrap().clear();

    let operation = f.service.game_create_save_snapshot(&f.game).await.unwrap();
    assert_eq!(operation.status, "completed");
    let inventory = FusionService::game_list_save_snapshots(&*f.storage, &f.game).unwrap();
    assert_eq!(inventory.snapshots.len(), 1);
    let id = &inventory.snapshots[0].id;
    let backup = f.service.library_root().join(format!(
        ".fusion-source/persistence/{}/{}/snapshots/{id}/workspace/saves/progress.dat",
        f.game.source_id, f.game.game_id
    ));
    assert_eq!(fs::read(backup).unwrap(), b"latest detached save");
    assert_eq!(active(&f).id, original.id);
    assert_eq!(fs::read(save).unwrap(), b"latest detached save");
    assert_eq!(
        fs::read(&original.persistence.bindings[0].persistent_path).unwrap(),
        b"initial save",
        "creating a backup must not overwrite the existing persistent slot"
    );
    assert!(f.transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn manual_backup_survives_missing_runtime_cache_without_enabling_launch() {
    use super::cas::CasStore;
    use super::layers::LayerInput;
    use super::workspace::WorkspaceAssembler;

    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let original = active(&f);
    let LayerInput::Cas { digest, .. } = &original.inputs["runtime"] else {
        panic!("fixture runtime must live in CAS");
    };
    let cas = CasStore::open(f.service.library_root()).unwrap();
    let runtime = cas.get(digest).unwrap();
    fs::rename(
        runtime.root.parent().unwrap(),
        f._temporary.path().join("unavailable-runtime-cache"),
    )
    .unwrap();

    let workspace = WorkspaceAssembler::open(f.service.library_root(), &f.game).unwrap();
    assert!(
        workspace.active().is_err(),
        "launch lease still requires CAS"
    );
    let result = f.service.game_create_save_snapshot(&f.game).await.unwrap();
    assert_eq!(result.status, "completed");
    let history = FusionService::game_list_save_snapshots(&*f.storage, &f.game).unwrap();
    assert_eq!(history.snapshots.len(), 1);
    let id = &history.snapshots[0].id;
    let backup = f.service.library_root().join(format!(
        ".fusion-source/persistence/{}/{}/snapshots/{id}/workspace/saves/progress.dat",
        f.game.source_id, f.game.game_id
    ));
    assert_eq!(fs::read(backup).unwrap(), b"initial save");
    assert_eq!(active(&f).id, original.id);
}

#[tokio::test]
async fn manual_backup_rejects_an_active_workspace_lease() {
    use super::workspace::WorkspaceAssembler;

    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let assembler = WorkspaceAssembler::open(f.service.library_root(), &f.game).unwrap();
    let lease = assembler.active().unwrap().unwrap();
    let error = f
        .service
        .game_create_save_snapshot(&f.game)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::OperationInProgress);
    assert!(
        FusionService::game_list_save_snapshots(&*f.storage, &f.game)
            .unwrap()
            .snapshots
            .is_empty()
    );
    drop(lease);
    assert_eq!(
        f.service
            .game_create_save_snapshot(&f.game)
            .await
            .unwrap()
            .status,
        "completed"
    );
}

#[tokio::test]
async fn manual_backup_does_not_publish_an_empty_save_directory() {
    let f = fixture();
    let mut wire = document("1", RUNTIME_ONE);
    wire["persistence"]["saves_paths"] = json!(["workspace/new-saves"]);
    f.transport.set(INDEX, index(&wire));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&wire).unwrap());
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let error = f
        .service
        .game_create_save_snapshot(&f.game)
        .await
        .unwrap_err();
    assert_eq!(error.details.get("reason"), Some(&json!("no_save_files")));
    assert!(
        FusionService::game_list_save_snapshots(&*f.storage, &f.game)
            .unwrap()
            .snapshots
            .is_empty()
    );
}

#[tokio::test]
async fn manual_backup_copies_an_owned_save_directory_but_refuses_its_replacement() {
    let f = fixture();
    let mut wire = document("1", RUNTIME_ONE);
    wire["persistence"]["saves_paths"] = json!(["workspace/saves"]);
    f.transport.set(INDEX, index(&wire));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&wire).unwrap());
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let save_dir = active(&f).workspace_root.join("saves");
    fs::write(save_dir.join("progress.dat"), b"current managed save").unwrap();
    f.service.game_create_save_snapshot(&f.game).await.unwrap();
    let before = FusionService::game_list_save_snapshots(&*f.storage, &f.game).unwrap();
    assert_eq!(before.snapshots.len(), 1);
    assert_eq!(before.snapshots[0].file_count, 1);
    super::persistence::unlink_workspace_save_link(&save_dir).unwrap();
    fs::create_dir(&save_dir).unwrap();
    fs::write(save_dir.join("progress.dat"), b"new unbound save").unwrap();

    assert_eq!(
        f.service
            .game_create_save_snapshot(&f.game)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PersistenceConflict
    );
    let after = FusionService::game_list_save_snapshots(&*f.storage, &f.game).unwrap();
    assert_eq!(after.snapshots.len(), 1);
    assert_eq!(after.snapshots[0].id, before.snapshots[0].id);
}

#[tokio::test]
async fn save_inventory_uses_only_the_active_qualified_installation_root() {
    use super::persistence::PersistenceManager;

    let f = fixture();
    let other_root = f._temporary.path().join("other-library");
    fs::create_dir(&other_root).unwrap();
    let stray = PersistenceManager::open(&other_root, &f.game).unwrap();
    stray.snapshot_before_update().unwrap();
    assert!(
        FusionService::game_list_save_snapshots(&*f.storage, &f.game)
            .unwrap()
            .snapshots
            .is_empty()
    );

    f.transport.set(INDEX, index(&document("1", RUNTIME_ONE)));
    f.transport.set(
        MANIFEST,
        serde_json::to_vec(&document("1", RUNTIME_ONE)).unwrap(),
    );
    f.service.source_add(INDEX).await.unwrap();
    let library = f._temporary.path().join("library");
    f.storage
        .bind_installation_root(&f.game, &library, None)
        .unwrap();
    let manager = PersistenceManager::open(&library, &f.game).unwrap();
    let first = manager.snapshot_before_update().unwrap();
    // Staging without an active installation.
    assert!(
        FusionService::game_list_save_snapshots(&*f.storage, &f.game)
            .unwrap()
            .snapshots
            .is_empty()
    );

    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let second = manager.snapshot_before_update().unwrap();
    let other_game = QualifiedGameId {
        source_id: "other-source".into(),
        game_id: f.game.game_id.clone(),
    };
    PersistenceManager::open(&library, &other_game)
        .unwrap()
        .snapshot_before_update()
        .unwrap();

    fs::remove_file(manager.root().join(".lock")).unwrap();
    let inventory = FusionService::game_list_save_snapshots(&*f.storage, &f.game).unwrap();
    let snapshots = &inventory.snapshots;
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].id, second.id);
    assert_eq!(snapshots[1].id, first.id);
    assert_eq!(snapshots[0].file_count, second.files.len());
    assert_eq!(snapshots[0].total_bytes, second.total_bytes);
    assert!(!manager.root().join(".lock").exists());
    assert!(chrono::DateTime::parse_from_rfc3339(&snapshots[0].created_at).is_ok());
    assert_eq!(
        serde_json::to_value(&inventory).unwrap(),
        json!({"snapshots": [
            {"id": second.id, "createdAt": snapshots[0].created_at,
             "fileCount": second.files.len(), "totalBytes": second.total_bytes},
            {"id": first.id, "createdAt": snapshots[1].created_at,
             "fileCount": first.files.len(), "totalBytes": first.total_bytes}
        ], "skippedCount": 0, "hasMore": false})
    );
}

fn signed_index(bytes: &[u8], key: &ed25519_dalek::SigningKey) -> Vec<u8> {
    let mut index: super::schema::SourceIndex = serde_json::from_slice(bytes).unwrap();
    super::signatures::sign_index(&mut index, key).unwrap();
    serde_json::to_vec_pretty(&index).unwrap()
}

#[tokio::test]
async fn signed_sources_pin_first_use_and_reject_tampering_and_reconnect_downgrades() {
    let f = fixture();
    let key = ed25519_dalek::SigningKey::from_bytes(&[31; 32]);
    let unsigned = index(&document("1", RUNTIME_ONE));
    let signed = signed_index(&unsigned, &key);
    f.transport.set(INDEX, signed.clone());
    assert!(
        f.service
            .source_preview(INDEX)
            .await
            .unwrap()
            .signature_verified
    );
    f.service.source_add(INDEX).await.unwrap();
    let original = f.storage.list_sources().unwrap()[0].clone();
    let pin = f
        .storage
        .pinned_source_identity("fixture-source", INDEX)
        .unwrap()
        .unwrap();
    assert_eq!(pin.public_key, original.index.source_info.public_key);
    for field in ["manifest_blake3", "manifest_url", "cover_url", "platform"] {
        let mut changed: Value = serde_json::from_slice(&signed).unwrap();
        changed["games"][0][field] = json!("tampered");
        f.transport
            .set(INDEX, serde_json::to_vec(&changed).unwrap());
        assert_eq!(
            f.service
                .source_refresh("fixture-source")
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidSourceSignature
        );
        assert_eq!(f.storage.list_sources().unwrap()[0].index, original.index);
    }
    f.service.source_remove("fixture-source").unwrap();
    assert!(f.storage.list_sources().unwrap().is_empty());
    assert_eq!(
        f.storage
            .pinned_source_identity("fixture-source", INDEX)
            .unwrap(),
        Some(pin)
    );
    // A new, valid self-signature cannot replace the independently stored pin.
    let replacement = ed25519_dalek::SigningKey::from_bytes(&[32; 32]);
    for rejected in [
        unsigned,
        signed_index(&index(&document("1", RUNTIME_ONE)), &replacement),
    ] {
        f.transport.set(INDEX, rejected);
        assert_eq!(
            f.service.source_preview(INDEX).await.unwrap_err().code,
            ErrorCode::InvalidSourceSignature
        );
        assert_eq!(
            f.service.source_add(INDEX).await.unwrap_err().code,
            ErrorCode::InvalidSourceSignature
        );
        assert!(f.storage.list_sources().unwrap().is_empty());
    }
    let mut stripped: Value = serde_json::from_slice(&signed).unwrap();
    stripped.as_object_mut().unwrap().remove("signature");
    f.transport
        .set(INDEX, serde_json::to_vec(&stripped).unwrap());
    assert_eq!(
        f.service.source_add(INDEX).await.unwrap_err().code,
        ErrorCode::InvalidSourceSignature
    );
    f.transport.set(INDEX, signed);
    f.service.source_add(INDEX).await.unwrap();
    assert_eq!(f.storage.list_sources().unwrap()[0].index, original.index);
}

#[tokio::test]
async fn strict_launch_checks_parent_signature_before_refreshing_any_catalog() {
    let f = fixture();
    let child_key = ed25519_dalek::SigningKey::from_bytes(&[41; 32]);
    let parent_key = ed25519_dalek::SigningKey::from_bytes(&[42; 32]);
    let parent_url = "https://parent.example/index.json";
    let parent_manifest_url = "https://parent.example/parent.json";
    let mut parent = document("1", RUNTIME_ONE);
    parent["id"] = json!("parent");
    parent.as_object_mut().unwrap().remove("runtime");
    parent["orchestrator"] = json!({"strict_sync":true,"processes":[{"id":"main","executable":"runtime/main.exe","role":"primary"}]});
    let parent_wire = json!({"schema_version":"1.0","source_info":{"id":"parent-source","name":"Parent"},"games":[{"id":"parent","title":"Parent","version":"1","manifest_url":"parent.json","manifest_blake3":hash(&serde_json::to_vec(&parent).unwrap())}]});
    let signed_parent = signed_index(&serde_json::to_vec(&parent_wire).unwrap(), &parent_key);
    let child = json!({"schema_version":"1.0","id":"native-game","title":"Child","version":"1","extends":"parent-source::parent"});
    let signed_child = signed_index(&index(&child), &child_key);
    f.transport.set(INDEX, signed_child.clone());
    f.transport
        .set(MANIFEST, serde_json::to_vec(&child).unwrap());
    f.transport.set(parent_url, signed_parent.clone());
    f.transport
        .set(parent_manifest_url, serde_json::to_vec(&parent).unwrap());
    f.service.source_add(INDEX).await.unwrap();
    f.service.source_add(parent_url).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    f.service
        .game_trust_executable(&f.game, "runtime/main.exe", &hash(RUNTIME_ONE))
        .await
        .unwrap();
    let original = f.storage.list_sources().unwrap();

    let mut next_child: super::schema::SourceIndex = serde_json::from_slice(&signed_child).unwrap();
    next_child.source_info.name = "Refreshed child name".into();
    super::signatures::sign_index(&mut next_child, &child_key).unwrap();
    f.transport
        .set(INDEX, serde_json::to_vec(&next_child).unwrap());
    let mut bad_parent: Value = serde_json::from_slice(&signed_parent).unwrap();
    bad_parent["games"][0]["manifest_url"] = json!("tampered.json");
    f.transport
        .set(parent_url, serde_json::to_vec(&bad_parent).unwrap());
    assert_eq!(
        f.service.game_launch(&f.game).await.unwrap_err().code,
        ErrorCode::InvalidSourceSignature
    );
    let after = f.storage.list_sources().unwrap();
    for (before, after) in original.iter().zip(&after) {
        assert_eq!(before.index, after.index);
        assert_eq!(before.refreshed_at, after.refreshed_at);
    }
    assert!(
        f.transport
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|url| url.as_str() == parent_url)
            .count()
            >= 2
    );
}

#[derive(Default)]
struct MemoryTransport {
    documents: Mutex<BTreeMap<String, Vec<u8>>>,
    calls: Mutex<Vec<String>>,
    stalled: Mutex<Option<String>>,
    operation_token: Mutex<Option<Cancellation>>,
}
impl MemoryTransport {
    fn set(&self, url: &str, value: Vec<u8>) {
        self.documents.lock().unwrap().insert(url.into(), value);
    }
}
#[async_trait]
impl ArtifactTransport for MemoryTransport {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.calls.lock().unwrap().push(url.into());
        *self.operation_token.lock().unwrap() = Some(cancel.clone());
        let stalled = self
            .stalled
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|stalled| stalled == url);
        if stalled {
            cancel.cancelled().await;
            return cancel.check().map(|()| unreachable!());
        }
        cancel.check()?;
        let bytes = self
            .documents
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| FusionError::new(ErrorCode::Network, "Offline fixture transport."))?;
        if bytes.len() as u64 > limits.max_bytes {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "Fixture too large.",
            ));
        }
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
}
#[derive(Default)]
struct Events(
    Mutex<Vec<OperationProgress>>,
    Mutex<Option<Arc<MemoryTransport>>>,
);
impl ProgressSink for Events {
    fn emit(&self, event: &OperationProgress) {
        self.0.lock().unwrap().push(event.clone());
        if event.phase == "transform" && event.layer_id.as_deref() == Some("runtime") {
            if let Some(transport) = self.1.lock().unwrap().take() {
                transport
                    .operation_token
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .cancel();
            }
        }
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    service: FusionService,
    storage: Arc<SqliteFusionStorage>,
    transport: Arc<MemoryTransport>,
    events: Arc<Events>,
    game: QualifiedGameId,
    request: InstallRequest,
}
fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
fn document(version: &str, runtime: &[u8]) -> Value {
    json!({"schema_version":"1.0","id":"native-game","title":"Integration fixture","version":version,"platform":"pc",
      "layers":[
       {"id":"base","kind":"base","source":{"kind":"user_provided"}},
       {"id":"runtime","kind":"runtime","depends_on":["base"],"mount_path":"runtime","source":{"kind":"user_provided"},"critical_files":[{"path":"main.exe","blake3":hash(runtime),"role":"main_executable"}]},
       {"id":"shader","kind":"shader_cache","optional":true,"source":{"kind":"user_provided"},"mount_path":"cache"}],
      "runtime":{"layer_id":"runtime","executable":"runtime/main.exe"},"persistence":{"saves_paths":["workspace/saves/progress.dat"]}})
}
fn index(document: &Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version":"1.0","source_info":{"id":"fixture-source","name":"Fixture source","telemetry_endpoint":"https://example.com/report"},"games":[{"id":"native-game","title":"Integration fixture","version":document["version"],"manifest_url":"game.json","manifest_blake3":hash(&serde_json::to_vec(document).unwrap())}]})).unwrap()
}
fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let library = root.join("library");
    let appdata = root.join("appdata");
    let local = root.join("local-appdata");
    let base = root.join("user-base");
    let runtime = root.join("user-runtime");
    for path in [&library, &appdata, &local, &base.join("saves"), &runtime] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(base.join("data.bin"), b"large immutable base fixture").unwrap();
    fs::write(base.join("saves/progress.dat"), b"initial save").unwrap();
    fs::write(runtime.join("main.exe"), RUNTIME_ONE).unwrap();
    let transport = Arc::new(MemoryTransport::default());
    let events = Arc::new(Events::default());
    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
        RepositoryStore::open(&root.join("test.db")).unwrap(),
    ))));
    let service = FusionService::open(
        &library,
        storage.clone(),
        transport.clone(),
        events.clone(),
        ServiceEnvironment {
            appdata,
            local_appdata: local,
            display_width: 1920,
            display_height: 1080,
            refresh_rate: 60,
        },
    )
    .unwrap();
    let game = QualifiedGameId {
        source_id: "fixture-source".into(),
        game_id: "native-game".into(),
    };
    let request = InstallRequest {
        user_files: BTreeMap::from([
            ("base".into(), base.to_string_lossy().into()),
            ("runtime".into(), runtime.to_string_lossy().into()),
        ]),
        ..InstallRequest::default()
    };
    let document = document("1", RUNTIME_ONE);
    transport.set(INDEX, index(&document));
    transport.set(MANIFEST, serde_json::to_vec(&document).unwrap());
    Fixture {
        _temporary: temporary,
        service,
        storage,
        transport,
        events,
        game,
        request,
    }
}
fn active(fixture: &Fixture) -> WorkspaceSnapshot {
    serde_json::from_str(
        &fixture
            .storage
            .get_active_snapshot(&fixture.game)
            .unwrap()
            .unwrap()
            .snapshot_json,
    )
    .unwrap()
}

#[tokio::test]
async fn catalog_batches_local_readiness_and_applied_options_without_fetching_manifests() {
    let f = fixture();
    let mut wire = document("1", RUNTIME_ONE);
    wire["cover_url"] = json!("https://example.com/cover.webp");
    wire["layers"][2]["title"] = json!("Кеш шейдеров");
    wire["layers"][2]["description"] = json!("Уменьшает задержки при первом запуске.");
    f.transport.set(INDEX, index(&wire));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&wire).unwrap());
    f.service.source_add(INDEX).await.unwrap();
    assert!(!f.service.source_list_catalog().unwrap()[0].ready_to_play);
    let first = f.service.game_get_state(&f.game).await.unwrap();
    assert!(first.applied_options.is_none());
    assert_eq!(
        first.default_runtime_channel,
        Some(super::schema::RuntimeChannel::Pinned)
    );
    assert_eq!(first.addons[0].title.as_deref(), Some("Кеш шейдеров"));
    assert_eq!(
        first.addons[0].description.as_deref(),
        Some("Уменьшает задержки при первом запуске.")
    );
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let untrusted = f.service.source_list_catalog().unwrap();
    assert!(untrusted[0].installed);
    assert!(!untrusted[0].ready_to_play);
    assert!(untrusted[0].active_snapshot_id.is_some());
    assert_eq!(untrusted[0].platform.as_deref(), Some("pc"));
    assert_eq!(
        untrusted[0].cover_url.as_deref(),
        Some("https://example.com/cover.webp")
    );
    f.storage
        .trust_executable(&f.game, "wrong/main.exe", &hash(RUNTIME_ONE))
        .unwrap();
    assert!(!f.service.source_list_catalog().unwrap()[0].ready_to_play);
    f.service
        .game_trust_executable(&f.game, "runtime/main.exe", &hash(RUNTIME_ONE))
        .await
        .unwrap();
    let calls = f.transport.calls.lock().unwrap().len();
    assert!(f.service.source_list_catalog().unwrap()[0].ready_to_play);
    assert_eq!(f.transport.calls.lock().unwrap().len(), calls);
    let changed = f
        .service
        .game_set_options(
            &f.game,
            GameOptions {
                enabled_addons: Some(["shader".into()].into_iter().collect()),
                runtime_channel: None,
                ..GameOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.options.enabled_addons.as_ref().unwrap().len(), 1);
    assert!(changed
        .applied_options
        .as_ref()
        .unwrap()
        .enabled_addons
        .as_ref()
        .unwrap()
        .is_empty());
    assert!(changed.external_workspace_root.is_none());
    // Desired settings and an unsupported current source do not disable the active snapshot.
    let mut newer = document("2", RUNTIME_TWO);
    newer["capabilities"] = json!({"required":["future.runtime.v99"]});
    f.transport.set(INDEX, index(&newer));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&newer).unwrap());
    f.service.source_refresh("fixture-source").await.unwrap();
    f.service.game_get_state(&f.game).await.unwrap();
    assert!(f.service.source_list_catalog().unwrap()[0].ready_to_play);
    let operation = f.storage.begin_operation(&f.game, "update").unwrap();
    assert!(!f.service.source_list_catalog().unwrap()[0].ready_to_play);
    f.storage
        .finish_operation(&operation.id, "completed")
        .unwrap();
    assert!(f.service.source_list_catalog().unwrap()[0].ready_to_play);
    let conn = rusqlite::Connection::open(f._temporary.path().join("test.db")).unwrap();
    conn.execute(
        "UPDATE fusion_workspace_snapshots SET snapshot_json=snapshot_json || ' '",
        [],
    )
    .unwrap();
    let damaged = f.service.source_list_catalog().unwrap();
    assert!(damaged[0].installed);
    assert!(!damaged[0].ready_to_play);
}

#[tokio::test]
async fn setup_lists_disabled_user_shader_without_selecting_it_or_saving_options() {
    let f = fixture();
    let mut wire = document("1", RUNTIME_ONE);
    wire["layers"][2]["title"] = json!("Shader cache");
    wire["layers"][2]["source"]["instructions"] = json!("Choose your own shader cache.");
    f.transport.set(INDEX, index(&wire));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&wire).unwrap());
    f.service.source_add(INDEX).await.unwrap();

    let state = f.service.game_get_state(&f.game).await.unwrap();
    assert_eq!(state.user_layers.len(), 3);
    assert!(state.options.enabled_addons.is_none());
    assert!(
        !state
            .addons
            .iter()
            .find(|addon| addon.id == "shader")
            .unwrap()
            .enabled
    );
    let serialized = serde_json::to_value(&state).unwrap();
    let requirements = serialized["userLayers"].as_array().unwrap();
    for id in ["base", "runtime"] {
        let requirement = requirements
            .iter()
            .find(|layer| layer["layerId"] == id)
            .unwrap();
        assert_eq!(requirement["optional"], json!(false));
        assert_eq!(requirement["title"], Value::Null);
        assert_eq!(requirement["rawPathRequired"], json!(true));
    }
    let shader = requirements
        .iter()
        .find(|layer| layer["layerId"] == "shader")
        .unwrap();
    assert_eq!(
        shader,
        &json!({
            "layerId": "shader",
            "title": "Shader cache",
            "optional": true,
            "instructions": "Choose your own shader cache.",
            "archive": "raw",
            "rawPathRequired": true
        })
    );

    // Setup metadata must not require a disabled shader's files during execution.
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    assert_eq!(active(&f).plan.selected_layers, vec!["base", "runtime"]);
    let installed = f.service.game_get_state(&f.game).await.unwrap();
    assert!(installed.options.enabled_addons.is_none());
    assert!(
        !installed
            .addons
            .iter()
            .find(|addon| addon.id == "shader")
            .unwrap()
            .enabled
    );
}

#[tokio::test]
async fn user_import_path_errors_identify_the_layer() {
    for relative_selection in [true, false] {
        let f = fixture();
        f.service.source_add(INDEX).await.unwrap();
        let mut request = f.request.clone();
        if relative_selection {
            request
                .user_files
                .insert("base".into(), "relative.bin".into());
        } else {
            request
                .raw_paths
                .insert("base".into(), "../escape.bin".into());
        }
        let error = f.service.game_install(&f.game, request).await.unwrap_err();
        assert_eq!(error.details.get("layerId"), Some(&json!("base")));
    }
}

#[tokio::test]
async fn user_import_digest_error_identifies_the_layer_and_preserves_digest_details() {
    let f = fixture();
    let mut wire = document("1", RUNTIME_ONE);
    let expected = hash(b"expected user-owned base");
    let actual = hash(b"wrong user-owned base");
    wire["layers"][0]["transport_digest"] = json!({
        "algorithm": "blake3",
        "value": expected
    });
    f.transport.set(INDEX, index(&wire));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&wire).unwrap());
    f.service.source_add(INDEX).await.unwrap();
    let file = f._temporary.path().join("wrong-base.bin");
    fs::write(&file, b"wrong user-owned base").unwrap();
    let mut request = f.request.clone();
    request
        .user_files
        .insert("base".into(), file.to_string_lossy().into());
    request.raw_paths.insert("base".into(), "data.bin".into());

    let error = f.service.game_install(&f.game, request).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::IntegrityViolation);
    assert_eq!(error.details.get("expected"), Some(&json!(expected)));
    assert_eq!(error.details.get("actual"), Some(&json!(actual)));
    assert_eq!(error.details.get("layerId"), Some(&json!("base")));
}

#[tokio::test]
async fn adding_source_only_fetches_compact_index_and_manifest_cache_is_hash_bound() {
    let f = fixture();
    let preview = f.service.source_preview(INDEX).await.unwrap();
    assert_eq!(preview.game_count, 1);
    assert!(!preview.signature_verified);
    f.service.source_add(INDEX).await.unwrap();
    assert!(f
        .transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|url| url == INDEX));
    assert!(!f.service.library_root().join(".fusion-source/cas").exists());
    let catalog = f.service.source_list_catalog().unwrap();
    assert_eq!(catalog[0].kind, "fusion_source");
    assert!(!catalog[0].installed);
    let first = f.service.game_get_state(&f.game).await.unwrap();
    assert!(!first.installed);
    assert_eq!(first.user_layers.len(), 3);
    assert_eq!(first.addons.len(), 1);
    f.service.game_get_state(&f.game).await.unwrap();
    assert_eq!(
        f.transport
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|url| url.as_str() == MANIFEST)
            .count(),
        1
    );
    let mut bad = document("2", RUNTIME_TWO);
    bad["title"] = json!("new version");
    f.transport.set(INDEX, index(&bad));
    f.transport.set(MANIFEST, b"tampered manifest".to_vec());
    f.service.source_refresh("fixture-source").await.unwrap();
    assert_eq!(
        f.service.game_get_state(&f.game).await.unwrap_err().code,
        ErrorCode::ManifestDigestMismatch
    );
}

#[tokio::test]
async fn install_exact_trust_runtime_update_and_rollback_keep_latest_saves() {
    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    let installed = f
        .service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    assert_eq!(installed.status, "completed");
    let initial = active(&f);
    let state = f.service.game_get_state(&f.game).await.unwrap();
    assert!(state.installed);
    assert!(!state.ready);
    assert_eq!(state.executables.len(), 1);
    assert_eq!(
        f.service
            .game_trust_executable(&f.game, "runtime/main.exe", &"a".repeat(64))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UntrustedExecutable
    );
    f.service
        .game_trust_executable(&f.game, "runtime/main.exe", &hash(RUNTIME_ONE))
        .await
        .unwrap();
    assert!(f.service.game_get_state(&f.game).await.unwrap().ready);
    fs::write(
        initial.workspace_root.join("saves/progress.dat"),
        b"latest save",
    )
    .unwrap();
    let runtime_path = Path::new(&f.request.user_files["runtime"]);
    fs::write(runtime_path.join("main.exe"), RUNTIME_TWO).unwrap();
    let next = document("2", RUNTIME_TWO);
    f.transport.set(INDEX, index(&next));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&next).unwrap());
    f.service.source_refresh("fixture-source").await.unwrap();
    let request = InstallRequest {
        user_files: BTreeMap::from([("runtime".into(), runtime_path.to_string_lossy().into())]),
        ..InstallRequest::default()
    };
    f.service.game_update(&f.game, request).await.unwrap();
    let updated = active(&f);
    assert!(updated.delta.reused.contains(&"base".into()));
    assert_eq!(
        fs::read(updated.workspace_root.join("saves/progress.dat")).unwrap(),
        b"latest save"
    );
    assert!(!f.service.game_get_state(&f.game).await.unwrap().ready);
    f.service.game_rollback(&f.game).await.unwrap();
    let rolled = active(&f);
    assert_eq!(rolled.id, initial.id);
    assert_eq!(
        fs::read(rolled.workspace_root.join("saves/progress.dat")).unwrap(),
        b"latest save"
    );
    assert!(f.service.game_get_state(&f.game).await.unwrap().ready);
    f.service.game_remove(&f.game).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    // SQLite intentionally retains old snapshot ownership history. It must not
    // make a fresh workspace advertise rollback to an uninstalled predecessor.
    assert!(
        !f.service
            .game_get_state(&f.game)
            .await
            .unwrap()
            .rollback_available
    );
    let events = f.events.0.lock().unwrap();
    for pair in events
        .windows(2)
        .filter(|pair| pair[0].operation_id == pair[1].operation_id)
    {
        assert!(pair[0].sequence < pair[1].sequence);
    }
}

#[tokio::test]
async fn corrupted_update_preserves_active_snapshot_and_offline_launch_readiness() {
    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    f.service
        .game_trust_executable(&f.game, "runtime/main.exe", &hash(RUNTIME_ONE))
        .await
        .unwrap();
    let initial = active(&f);
    let next = document("2", RUNTIME_TWO);
    f.transport.set(INDEX, index(&next));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&next).unwrap());
    f.service.source_refresh("fixture-source").await.unwrap();
    assert!(f
        .service
        .game_update(&f.game, f.request.clone())
        .await
        .is_err());
    assert_eq!(active(&f).id, initial.id);
    // Advertise a never-cached version, then lose its manifest server. The old
    // trusted snapshot stays launchable while the update is merely unavailable.
    let uncached = document("3", RUNTIME_TWO);
    f.transport.set(INDEX, index(&uncached));
    f.service.source_refresh("fixture-source").await.unwrap();
    f.transport.documents.lock().unwrap().remove(MANIFEST);
    let state = f.service.game_get_state(&f.game).await.unwrap();
    assert!(state.ready);
    assert!(!state.warnings.is_empty());
    assert_eq!(
        f.service.source_remove("fixture-source").unwrap_err().code,
        ErrorCode::SnapshotConflict
    );
    f.service.game_remove(&f.game).await.unwrap();
    assert!(!f.service.source_list_catalog().unwrap()[0].installed);
    f.service.source_remove("fixture-source").unwrap();
    assert!(f.service.source_list_catalog().unwrap().is_empty());
}

#[tokio::test]
async fn cancel_at_remote_zip_transform_never_imports_runtime_into_cas() {
    let f = fixture();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("main.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(RUNTIME_ONE).unwrap();
    let zip = zip.finish().unwrap().into_inner();
    let archive = "https://example.com/source/runtime.zip";
    let mut wire = document("1", RUNTIME_ONE);
    wire["layers"][1]["source"] = json!({"kind":"remote","mirrors":[archive],"archive":"zip"});
    wire["layers"][1]["transport_digest"] = json!({"algorithm":"blake3","value":hash(&zip)});
    f.transport.set(archive, zip);
    f.transport.set(INDEX, index(&wire));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&wire).unwrap());
    f.service.source_add(INDEX).await.unwrap();
    *f.events.1.lock().unwrap() = Some(f.transport.clone());
    let mut request = f.request.clone();
    request.user_files.remove("runtime");
    assert_eq!(
        f.service
            .game_install(&f.game, request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(
        f.events.1.lock().unwrap().is_none(),
        "cancellation at the transform boundary did not run"
    );
    assert!(f.storage.get_active_snapshot(&f.game).unwrap().is_none());
    let cas = f.service.library_root().join(".fusion-source/cas");
    assert_eq!(
        fs::read_dir(cas.join("objects")).unwrap().count(),
        1,
        "only the preceding base layer may be imported"
    );
    assert_eq!(fs::read_dir(cas.join("staging")).unwrap().count(), 0);
}

#[tokio::test]
async fn cancel_stalled_transport_releases_operation_and_cannot_activate() {
    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    *f.transport.stalled.lock().unwrap() = Some(MANIFEST.into());
    let service = f.service.clone();
    let game = f.game.clone();
    let request = f.request.clone();
    let pending = tokio::spawn(async move { service.game_install(&game, request).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if f.service.has_active_operations() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.service
            .game_install(&f.game, f.request.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::OperationInProgress
    );
    f.service.game_cancel(&f.game).unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert!(!f.service.has_active_operations());
    assert!(f.storage.get_active_snapshot(&f.game).unwrap().is_none());
    assert_eq!(
        f.events.0.lock().unwrap().last().unwrap().status,
        "cancelled"
    );
}

#[tokio::test]
async fn crash_preview_and_export_only_use_locally_sanitized_report() {
    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let snapshot = active(&f);
    let sanitizer = TelemetrySanitizer::for_snapshot(&snapshot);
    let report = CrashReportBuilder::build(
        &snapshot,
        CrashOutcome {
            primary_exit_code: 17,
            user_cancelled: false,
        },
        &format!(
            "Failure at {} with Bearer secret-token",
            snapshot.workspace_root.display()
        ),
        &sanitizer,
    )
    .unwrap()
    .unwrap();
    let stored = f.storage.put_crash(&report).unwrap();
    let state = f.service.game_get_state(&f.game).await.unwrap();
    assert_eq!(state.crash_report_ids, vec![stored.id.clone()]);
    assert_eq!(state.crash_reports[0].report_id, stored.id);
    assert_eq!(state.crash_reports[0].created_at, stored.created_at);
    assert_eq!(state.crash_reports[0].exit_code, Some(17));
    assert_eq!(
        state.crash_reports[0].crash_signature_hash.as_deref(),
        stored.report["crashSignatureHash"].as_str()
    );
    let preview = f.service.crash_preview(&stored.id).unwrap();
    assert!(preview.can_send);
    let export = f.service.crash_send_target(&stored.id).unwrap();
    let body = String::from_utf8(export.body().to_vec()).unwrap();
    assert!(!body.contains(&snapshot.workspace_root.to_string_lossy().to_string()));
    assert!(!body.contains("secret-token"));
    assert_eq!(export.endpoint(), "https://example.com/report");
    f.service.crash_mark_sent(&stored.id).unwrap();
    assert!(!f.service.crash_preview(&stored.id).unwrap().can_send);
    assert!(f.service.crash_send_target(&stored.id).is_err());
}

#[tokio::test]
async fn activation_and_uninstall_crash_boundaries_reconcile_from_owned_filesystem_state() {
    let f = fixture();
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    let installed = active(&f);
    // Model a crash after the atomic FS switch and before SQLite activation.
    f.storage.remove_installation(&f.game).unwrap();
    f.storage
        .bind_installation_root(&f.game, f.service.library_root(), None)
        .unwrap();
    assert_eq!(
        f.storage.get_installation(&f.game).unwrap().unwrap().status,
        "staging"
    );
    let repaired = f.service.game_get_state(&f.game).await.unwrap();
    assert!(repaired.installed);
    assert_eq!(
        repaired.active_snapshot_id.as_deref(),
        Some(installed.id.as_str())
    );
    // Model a crash after committed FS uninstall and before SQLite removal.
    super::workspace::WorkspaceAssembler::open(f.service.library_root(), &f.game)
        .unwrap()
        .remove_installation()
        .unwrap();
    assert!(f.storage.get_active_snapshot(&f.game).unwrap().is_some());
    assert!(!f.service.game_get_state(&f.game).await.unwrap().installed);
    assert!(f.storage.get_active_snapshot(&f.game).unwrap().is_none());
}

#[tokio::test]
async fn source_extends_are_resolved_on_demand_and_cycles_block_only_the_requested_game() {
    let f = fixture();
    let mut base = document("1", RUNTIME_ONE);
    base["id"] = json!("parent");
    let child = json!({"schema_version":"1.0","id":"native-game","title":"Inherited game","version":"1","extends":"parent"});
    let mut source: Value = serde_json::from_slice(&index(&child)).unwrap();
    source["games"].as_array_mut().unwrap().push(json!({"id":"parent","title":"Base","version":"1","manifest_url":"parent.json","manifest_blake3":hash(&serde_json::to_vec(&base).unwrap())}));
    f.transport.set(INDEX, serde_json::to_vec(&source).unwrap());
    f.transport
        .set(MANIFEST, serde_json::to_vec(&child).unwrap());
    f.transport.set(
        "https://example.com/source/parent.json",
        serde_json::to_vec(&base).unwrap(),
    );
    f.service.source_add(INDEX).await.unwrap();
    let state = f.service.game_get_state(&f.game).await.unwrap();
    assert_eq!(state.user_layers.len(), 3);
    base["extends"] = json!("native-game");
    source["games"][1]["manifest_blake3"] = json!(hash(&serde_json::to_vec(&base).unwrap()));
    f.transport.set(INDEX, serde_json::to_vec(&source).unwrap());
    f.transport.set(
        "https://example.com/source/parent.json",
        serde_json::to_vec(&base).unwrap(),
    );
    f.service.source_refresh("fixture-source").await.unwrap();
    assert_eq!(
        f.service.game_get_state(&f.game).await.unwrap_err().code,
        ErrorCode::ExtendsCycle
    );
    assert_eq!(f.service.source_list_catalog().unwrap().len(), 2);
}

#[tokio::test]
async fn recovery_inspection_holds_the_workspace_lock_for_empty_and_active_states() {
    let f = fixture();
    let assembler =
        super::workspace::WorkspaceAssembler::open(f.service.library_root(), &f.game).unwrap();
    assembler
        .inspect_active(|lease, can_rollback| {
            assert!(lease.is_none());
            assert!(!can_rollback);
            assert_eq!(
                assembler.active().err().unwrap().code,
                ErrorCode::OperationInProgress
            );
            Ok(())
        })
        .unwrap();
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    assembler
        .inspect_active(|lease, can_rollback| {
            assert!(lease.is_some());
            assert!(!can_rollback);
            assert_eq!(
                assembler.active().err().unwrap().code,
                ErrorCode::OperationInProgress
            );
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn update_repairs_a_missing_writable_injector_without_losing_saves_or_early_activation() {
    let f = fixture();
    let injector = b"fixture injector DLL";
    fs::write(
        Path::new(&f.request.user_files["runtime"]).join("injector.dll"),
        injector,
    )
    .unwrap();
    let mut manifest = document("1", RUNTIME_ONE);
    manifest["layers"][1]["writable_copy_paths"] = json!(["injector.dll"]);
    manifest["layers"][1]["critical_files"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":"injector.dll","blake3":hash(injector),"role":"injector_dll"}));
    f.transport.set(INDEX, index(&manifest));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&manifest).unwrap());
    f.service.source_add(INDEX).await.unwrap();
    f.service
        .game_install(&f.game, f.request.clone())
        .await
        .unwrap();
    f.service
        .game_trust_executable(&f.game, "runtime/main.exe", &hash(RUNTIME_ONE))
        .await
        .unwrap();
    f.service
        .game_trust_executable(&f.game, "runtime/injector.dll", &hash(injector))
        .await
        .unwrap();
    let initial = active(&f);
    let cached = super::layers::LayerCache::open(f.service.library_root())
        .unwrap()
        .object(&initial.inputs["runtime"])
        .unwrap();
    assert!(cached
        .root
        .join("injector.dll")
        .metadata()
        .unwrap()
        .permissions()
        .readonly());
    fs::write(
        initial.workspace_root.join("saves/progress.dat"),
        b"save after last launch",
    )
    .unwrap();
    fs::remove_file(initial.workspace_root.join("runtime/injector.dll")).unwrap();
    let state = f.service.game_get_state(&f.game).await.unwrap();
    assert!(state.installed);
    assert!(!state.ready);
    assert!(state
        .blockers
        .iter()
        .any(|error| error.code == ErrorCode::MissingInjectorDll));
    assert_eq!(
        f.service.game_launch(&f.game).await.unwrap_err().code,
        ErrorCode::MissingInjectorDll
    );

    // A broken replacement must not activate merely because the old snapshot
    // already needs repair. Its current saves remain outside both candidates.
    let mut broken = manifest.clone();
    broken["version"] = json!("2");
    broken["config_templates"] =
        json!([{"id":"invalid","path":"settings.ini","format":"ini","template":"[unfinished"}]);
    f.transport.set(INDEX, index(&broken));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&broken).unwrap());
    f.service.source_refresh("fixture-source").await.unwrap();
    assert_eq!(
        f.service
            .game_update(&f.game, InstallRequest::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidTemplate
    );
    assert_eq!(active(&f).id, initial.id);
    assert_eq!(
        fs::read(initial.workspace_root.join("saves/progress.dat")).unwrap(),
        b"save after last launch"
    );

    manifest["version"] = json!("2");
    f.transport.set(INDEX, index(&manifest));
    f.transport
        .set(MANIFEST, serde_json::to_vec(&manifest).unwrap());
    f.service.source_refresh("fixture-source").await.unwrap();
    f.service
        .game_update(&f.game, InstallRequest::default())
        .await
        .unwrap();
    let repaired = active(&f);
    assert_ne!(repaired.id, initial.id);
    assert!(repaired.delta.reused.contains(&"base".to_owned()));
    assert!(repaired.delta.reused.contains(&"runtime".to_owned()));
    assert_eq!(
        fs::read(repaired.workspace_root.join("runtime/injector.dll")).unwrap(),
        injector
    );
    assert_eq!(
        fs::read(cached.root.join("injector.dll")).unwrap(),
        injector
    );
    assert_eq!(
        fs::read(repaired.workspace_root.join("saves/progress.dat")).unwrap(),
        b"save after last launch"
    );
    assert!(f.service.game_get_state(&f.game).await.unwrap().ready);
}

#[tokio::test]
async fn cross_source_inheritance_resolves_relative_mirrors_from_declaring_source_root() {
    let f = fixture();
    let parent_url = "https://parent.example/root-a/source-index.json";
    let child_url = "https://child.example/root-b/source-index.json";
    let parent_runtime = b"parent-runtime-bytes";
    let child_addon = b"child-addon-bytes";

    let descriptor = |id: &str, title: &str| {
        json!({
            "program_descriptor": {
                "document_type": "program_descriptor",
                "schema_version": "1.0",
                "identity": {
                    "id": id,
                    "title": title,
                    "version": "1.0",
                    "os": "windows",
                    "architecture": "x86_64"
                },
                "executor": {"id": "fusion.declarative", "version": "1.0"},
                "components": [
                    {
                        "id": "program",
                        "layer_id": "runtime",
                        "origin": {"kind": "managed", "artifact_ref": "program", "redistribution_allowed": true},
                        "source_root": "",
                        "mount_path": "runtime"
                    },
                    {
                        "id": "addon",
                        "layer_id": "addon",
                        "origin": {"kind": "managed", "artifact_ref": "addon", "redistribution_allowed": true},
                        "source_root": "",
                        "mount_path": "addons"
                    }
                ],
                "required_files": [
                    {"path": "runtime/main.exe", "role": "main_executable"}
                ],
                "projection_rules": [],
                "directories": [],
                "entrypoint": {
                    "executable": "runtime/main.exe",
                    "working_directory": "runtime",
                    "arguments": [],
                    "environment_paths": []
                },
                "configs": [],
                "persistence": [],
                "build_profiles": [{
                    "id": "full",
                    "title": "Full",
                    "layer_ids": ["runtime", "addon"],
                    "controls_profile_ids": ["native"],
                    "settings": []
                }],
                "controls_profiles": [{
                    "id": "native",
                    "title": "Native",
                    "kind": "application_defaults"
                }]
            }
        })
    };

    let parent_doc = json!({
        "schema_version": "1.1",
        "id": "parent-game",
        "title": "Parent 1.1",
        "version": "1.0",
        "capabilities": {"required": [], "optional": []},
        "layers": [{
            "id": "runtime",
            "kind": "runtime",
            "mount_path": "runtime",
            "source": {"kind": "remote", "mirrors": ["artifacts/parent-main.exe"], "archive": "raw"},
            "transport_digest": {"algorithm": "blake3", "value": hash(parent_runtime)},
            "size_bytes": parent_runtime.len(),
            "critical_files": [{"path": "main.exe", "blake3": hash(parent_runtime), "role": "main_executable"}],
            "default_enabled": true
        }],
        "launch": {
            "id": "program",
            "executable": "runtime/main.exe",
            "args": [],
            "env": {},
            "depends_on": [],
            "role": "primary"
        },
        "persistence": {"saves_paths": []}
    });
    let parent_raw = serde_json::to_vec(&parent_doc).unwrap();
    let parent_index = json!({
        "schema_version": "1.0",
        "source_info": {"id": "source-a", "name": "Source A"},
        "games": [{
            "id": "parent-game",
            "title": "Parent 1.1",
            "version": "1.0",
            "manifest_url": "manifests/nested/parent.json",
            "manifest_blake3": hash(&parent_raw)
        }]
    });

    let child_doc = json!({
        "schema_version": "1.1",
        "id": "child-game",
        "title": "Child 1.1",
        "version": "1.0",
        "extends": "source-a::parent-game",
        "creator": descriptor("child-game", "Child 1.1"),
        "layers": [{
            "id": "addon",
            "kind": "addon",
            "mount_path": "addons",
            "depends_on": ["runtime"],
            "source": {"kind": "remote", "mirrors": ["artifacts/child-extra.bin"], "archive": "raw"},
            "transport_digest": {"algorithm": "blake3", "value": hash(child_addon)},
            "size_bytes": child_addon.len(),
            "default_enabled": true
        }]
    });
    let child_raw = serde_json::to_vec(&child_doc).unwrap();
    let child_index = json!({
        "schema_version": "1.0",
        "source_info": {"id": "source-b", "name": "Source B"},
        "games": [{
            "id": "child-game",
            "title": "Child 1.1",
            "version": "1.0",
            "manifest_url": "manifests/nested/child.json",
            "manifest_blake3": hash(&child_raw)
        }]
    });

    f.transport
        .set(parent_url, serde_json::to_vec(&parent_index).unwrap());
    f.transport.set(
        "https://parent.example/root-a/manifests/nested/parent.json",
        parent_raw,
    );
    f.transport.set(
        "https://parent.example/root-a/artifacts/parent-main.exe",
        parent_runtime.to_vec(),
    );
    f.transport
        .set(child_url, serde_json::to_vec(&child_index).unwrap());
    f.transport.set(
        "https://child.example/root-b/manifests/nested/child.json",
        child_raw,
    );
    f.transport.set(
        "https://child.example/root-b/artifacts/child-extra.bin",
        child_addon.to_vec(),
    );

    f.service.source_add(parent_url).await.unwrap();
    f.service.source_add(child_url).await.unwrap();
    let game = QualifiedGameId {
        source_id: "source-b".into(),
        game_id: "child-game".into(),
    };
    f.service
        .game_set_options(
            &game,
            GameOptions {
                build_profile_id: Some("full".into()),
                controls_profile_id: Some("native".into()),
                ..GameOptions::default()
            },
        )
        .await
        .unwrap();
    let status = f
        .service
        .game_install(
            &game,
            InstallRequest {
                raw_paths: BTreeMap::from([
                    ("runtime".into(), "main.exe".into()),
                    ("addon".into(), "extra.bin".into()),
                ]),
                ..InstallRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(status.status, "completed");

    // 1.1 child extending a 1.0 parent with an absolute mirror also resolves cleanly.
    let mut legacy_parent = parent_doc;
    legacy_parent["schema_version"] = json!("1.0");
    legacy_parent["id"] = json!("parent-10");
    legacy_parent["layers"][0]["source"]["mirrors"] =
        json!(["https://parent.example/cdn/parent-10.exe"]);
    let legacy_raw = serde_json::to_vec(&legacy_parent).unwrap();
    let child11_raw = serde_json::to_vec(&json!({
        "schema_version": "1.1",
        "id": "child-from-10",
        "title": "Child from 1.0",
        "version": "1.0",
        "extends": "source-a::parent-10",
        "creator": descriptor("child-from-10", "Child from 1.0"),
        "layers": [{
            "id": "addon",
            "kind": "addon",
            "mount_path": "addons",
            "depends_on": ["runtime"],
            "source": {"kind": "remote", "mirrors": ["artifacts/child-from-10.bin"], "archive": "raw"},
            "transport_digest": {"algorithm": "blake3", "value": hash(child_addon)},
            "size_bytes": child_addon.len(),
            "default_enabled": true
        }]
    }))
    .unwrap();
    f.transport.set("https://parent.example/root-a/source-index.json", serde_json::to_vec(&json!({
        "schema_version": "1.0",
        "source_info": {"id": "source-a", "name": "Source A"},
        "games": [{"id": "parent-10", "title": "Parent 1.0", "version": "1.0", "manifest_url": "manifests/nested/p10.json", "manifest_blake3": hash(&legacy_raw)}]
    })).unwrap());
    f.transport.set(
        "https://parent.example/root-a/manifests/nested/p10.json",
        legacy_raw,
    );
    f.transport.set(
        "https://parent.example/cdn/parent-10.exe",
        parent_runtime.to_vec(),
    );
    f.service.source_refresh("source-a").await.unwrap();
    f.transport.set(child_url, serde_json::to_vec(&json!({
        "schema_version": "1.0",
        "source_info": {"id": "source-b", "name": "Source B"},
        "games": [{"id": "child-from-10", "title": "Child from 1.0", "version": "1.0", "manifest_url": "manifests/nested/c10.json", "manifest_blake3": hash(&child11_raw)}]
    })).unwrap());
    f.transport.set(
        "https://child.example/root-b/manifests/nested/c10.json",
        child11_raw,
    );
    f.transport.set(
        "https://child.example/root-b/artifacts/child-from-10.bin",
        child_addon.to_vec(),
    );
    f.service.source_refresh("source-b").await.unwrap();
    let game2 = QualifiedGameId {
        source_id: "source-b".into(),
        game_id: "child-from-10".into(),
    };
    f.service
        .game_set_options(
            &game2,
            GameOptions {
                build_profile_id: Some("full".into()),
                controls_profile_id: Some("native".into()),
                ..GameOptions::default()
            },
        )
        .await
        .unwrap();
    let status2 = f
        .service
        .game_install(
            &game2,
            InstallRequest {
                raw_paths: BTreeMap::from([
                    ("runtime".into(), "main.exe".into()),
                    ("addon".into(), "extra.bin".into()),
                ]),
                ..InstallRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(status2.status, "completed");

    let calls = f.transport.calls.lock().unwrap().clone();
    for expected in [
        "https://parent.example/root-a/artifacts/parent-main.exe",
        "https://child.example/root-b/artifacts/child-extra.bin",
        "https://parent.example/cdn/parent-10.exe",
        "https://child.example/root-b/artifacts/child-from-10.bin",
    ] {
        assert!(
            calls.contains(&expected.to_owned()),
            "missing expected {expected} in calls: {calls:?}"
        );
    }
    assert!(
        !calls
            .iter()
            .any(|url| url.contains("/manifests/nested/artifacts/"))
            && !calls
                .contains(&"https://child.example/root-b/artifacts/parent-main.exe".to_owned()),
        "relative mirror resolved from wrong base: {calls:?}"
    );
}
