//! Acquisition tests use the real HTTPS adapter and existing service/normalizer/CAS.
use super::*;
use crate::fusion_source::adapters::SecureHttpsTransport;
use crate::fusion_source::dto::{GameOptions, InstallRequest};
use crate::fusion_source::service::{FusionService, FusionStorage, NoProgress, ServiceEnvironment};
use crate::fusion_source::test_support::https::HttpsFixture;
use crate::fusion_source::workspace::WorkspaceSnapshot;
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct Fixture {
    service: FusionService,
    storage: Arc<SqliteFusionStorage>,
    server: HttpsFixture,
    library: PathBuf,
    root: tempfile::TempDir,
    document: Value,
    inventory: String,
    game: QualifiedGameId,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let selected = root.path().join("selected");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("program.exe"), b"program bytes").unwrap();
        let inventory = CasStore::open(&root.path().join("reference-only"))
            .unwrap()
            .import_directory(&selected)
            .unwrap()
            .digest;
        let archive = fs::File::create(root.path().join("payload.zip")).unwrap();
        let mut zip = zip::ZipWriter::new(archive);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("bundle/program.exe", options).unwrap();
        zip.write_all(b"program bytes").unwrap();
        zip.start_file("outside/readme.txt", options).unwrap();
        zip.write_all(b"not selected").unwrap();
        zip.finish().unwrap();
        let transport = crate::fusion_source::cas::hash_file(
            &root.path().join("payload.zip"),
            crate::fusion_source::schema::DigestAlgorithm::Blake3,
        )
        .unwrap();
        let server = HttpsFixture::start(root.path());
        let canonical = manifest();
        let creator = serde_json::to_value(canonical.creator.unwrap()).unwrap();
        let layer: crate::fusion_source::schema::LayerSpec =
            canonical.layers["runtime"].clone().into();
        let mut document = json!({"schema_version":"1.1", "id":"generic", "title":"Generic", "version":"1", "creator":creator, "layers":[layer]});
        document["creator"]["program_descriptor"]["components"][0]["source_root"] = "bundle".into();
        document["creator"]["program_descriptor"]["components"][0]["origin"] = json!({
            "kind":"external", "url":server.url("payload"), "transport_blake3":transport, "inventory_blake3":inventory
        });
        let library = root.path().join("library");
        let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
            RepositoryStore::open(&root.path().join("player.sqlite")).unwrap(),
        ))));
        let service = FusionService::open(
            &library,
            storage.clone(),
            Arc::new(SecureHttpsTransport),
            Arc::new(NoProgress),
            ServiceEnvironment {
                appdata: root.path().join("appdata"),
                local_appdata: root.path().join("local"),
                display_width: 1280,
                display_height: 720,
                refresh_rate: 60,
            },
        )
        .unwrap();
        let game = QualifiedGameId {
            source_id: "external-fixture".into(),
            game_id: "generic".into(),
        };
        let fixture = Self {
            service,
            storage,
            server,
            library,
            root,
            document,
            inventory,
            game,
        };
        fixture.publish();
        fixture
    }
    fn publish(&self) {
        let bytes = serde_json::to_vec(&self.document).unwrap();
        fs::write(self.root.path().join("game.json"), &bytes).unwrap();
        let mut index: crate::fusion_source::schema::SourceIndex = serde_json::from_value(json!({
            "schema_version":"1.0", "source_info":{"id":"external-fixture", "name":"External fixture"},
            "games":[{"id":"generic", "title":"Generic", "version":self.document["version"], "manifest_url":self.server.url("game.json"), "manifest_blake3":blake3::hash(&bytes).to_hex().to_string()}]
        })).unwrap();
        crate::fusion_source::signatures::sign_index(
            &mut index,
            &ed25519_dalek::SigningKey::from_bytes(&[53; 32]),
        )
        .unwrap();
        fs::write(
            self.root.path().join("index.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
    }
    async fn connect(&self) {
        self.service
            .source_add(&self.server.url("index.json"))
            .await
            .unwrap();
        self.storage
            .set_options(
                &self.game,
                &GameOptions {
                    build_profile_id: Some("standard".into()),
                    controls_profile_id: Some("native".into()),
                    ..GameOptions::default()
                },
            )
            .unwrap();
    }
    fn active(&self) -> WorkspaceSnapshot {
        serde_json::from_str(
            &self
                .storage
                .get_active_snapshot(&self.game)
                .unwrap()
                .unwrap()
                .snapshot_json,
        )
        .unwrap()
    }
}

#[tokio::test]
async fn external_https_clean_cache_acquires_normalized_cas_and_installs_without_user_files() {
    let fixture = Fixture::new();
    fixture.connect().await;
    assert!(
        fs::read_dir(fixture.library.join(".fusion-source/cas/objects"))
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    );
    fixture
        .service
        .game_install(&fixture.game, InstallRequest::default())
        .await
        .unwrap();
    let snapshot = fixture.active();
    assert!(
        matches!(&snapshot.inputs["runtime"], LayerInput::Cas { digest, runtime_lock:None } if digest == &fixture.inventory)
    );
    assert_eq!(snapshot.plan.selected_layers, ["runtime"]);
    assert_eq!(
        fs::read(snapshot.workspace_root.join("app/program.exe")).unwrap(),
        b"program bytes"
    );
    assert!(!snapshot.workspace_root.join("app/bundle").exists());
    assert!(!snapshot.workspace_root.join("app/outside").exists());
    CasStore::open(&fixture.library)
        .unwrap()
        .verify(&fixture.inventory)
        .unwrap();
}

#[tokio::test]
async fn external_state_omits_local_input_requirements_without_allowing_overrides() {
    let mut fixture = Fixture::new();
    fixture.connect().await;
    assert_eq!(
        fixture.document["layers"][0]["source"]["kind"],
        "user_provided"
    );
    let state = fixture.service.game_get_state(&fixture.game).await.unwrap();
    assert!(
        state.user_layers.is_empty(),
        "descriptor-owned external acquisition must not request local input: {:?}",
        state.user_layers
    );
    for raw_only in [false, true] {
        let mut request = InstallRequest::default();
        if raw_only {
            request
                .raw_paths
                .insert("runtime".into(), "program.exe".into());
        } else {
            request.user_files.insert(
                "runtime".into(),
                fixture
                    .root
                    .path()
                    .join("selected")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        let error = fixture
            .service
            .game_install(&fixture.game, request)
            .await
            .unwrap_err();
        assert_eq!(error.code, crate::fusion_source::ErrorCode::InvalidManifest);
        assert!(error
            .message
            .contains("cannot be replaced by a user import"));
        assert!(fixture
            .storage
            .get_active_snapshot(&fixture.game)
            .unwrap()
            .is_none());
    }
    fixture.document["creator"]["program_descriptor"]["components"][0]["origin"] = json!({
        "kind":"managed", "artifact_ref":"runtime", "redistribution_allowed":false
    });
    fixture.publish();
    fixture
        .service
        .source_refresh(&fixture.game.source_id)
        .await
        .unwrap();
    let state = fixture.service.game_get_state(&fixture.game).await.unwrap();
    assert_eq!(
        state.user_layers.len(),
        1,
        "managed user imports must remain visible"
    );
    assert_eq!(state.user_layers[0].layer_id, "runtime");
    assert_eq!(state.user_layers[0].archive, "raw");
    assert!(state.user_layers[0].raw_path_required);
}

fn workspace_bytes(
    root: &std::path::Path,
) -> Vec<(
    String,
    crate::fusion_source::fs_safety::FileIdentity,
    String,
)> {
    crate::fusion_source::fs_safety::list_plain_files(root)
        .unwrap()
        .into_iter()
        .map(|(name, path)| {
            (
                name,
                crate::fusion_source::fs_safety::file_identity(&path).unwrap(),
                crate::fusion_source::cas::hash_file(
                    &path,
                    crate::fusion_source::schema::DigestAlgorithm::Blake3,
                )
                .unwrap(),
            )
        })
        .collect()
}

async fn rejected_update_preserves_installation(case: &str) {
    use crate::fusion_source::ErrorCode;
    let mut fixture = Fixture::new();
    fixture.connect().await;
    fixture
        .service
        .game_install(&fixture.game, InstallRequest::default())
        .await
        .unwrap();
    let active = fixture.active();
    fs::write(
        active.workspace_root.join("userdata/save.dat"),
        b"latest private progress",
    )
    .unwrap();
    let before = workspace_bytes(&active.workspace_root);
    let before_record = fixture
        .storage
        .get_active_snapshot(&fixture.game)
        .unwrap()
        .unwrap()
        .snapshot_json;
    let original = fixture.document.clone();
    fixture.document["version"] = "2".into();
    let origin = &mut fixture.document["creator"]["program_descriptor"]["components"][0]["origin"];
    let expected = match case {
        "http" => {
            origin["url"] = "http://acquisition.test/payload".into();
            None
        }
        "credentials" => {
            origin["url"] = "https://user:password@acquisition.test/payload".into();
            None
        }
        "transport" => {
            origin["transport_blake3"] = "a".repeat(64).into();
            Some(ErrorCode::TransportDigestMismatch)
        }
        "inventory" => {
            origin["inventory_blake3"] = "b".repeat(64).into();
            Some(ErrorCode::IntegrityViolation)
        }
        "truncated" => {
            origin["url"] = fixture.server.url("truncated").into();
            Some(ErrorCode::Network)
        }
        "eof" => {
            origin["url"] = fixture.server.url("eof").into();
            Some(ErrorCode::Network)
        }
        "oversized" => {
            origin["url"] = fixture.server.url("oversized").into();
            Some(ErrorCode::TransferLimitExceeded)
        }
        "redirect" => {
            origin["url"] = fixture.server.url("redirect").into();
            Some(ErrorCode::Network)
        }
        "source-root" => {
            fixture.document["creator"]["program_descriptor"]["components"][0]["source_root"] =
                "missing".into();
            Some(ErrorCode::InvalidPath)
        }
        _ => panic!("unknown negative case"),
    };
    fixture.publish();
    fixture
        .service
        .source_refresh(&fixture.game.source_id)
        .await
        .unwrap();
    let error = fixture
        .service
        .game_update(&fixture.game, InstallRequest::default())
        .await
        .unwrap_err();
    if let Some(expected) = expected {
        assert_eq!(error.code, expected, "{case}: {error:?}");
    }
    assert_ne!(
        error.code,
        ErrorCode::ActionUnavailable,
        "{case}: not a gate-only rejection"
    );
    assert_eq!(
        workspace_bytes(&active.workspace_root),
        before,
        "{case}: active bytes or identities changed"
    );
    assert_eq!(
        fixture
            .storage
            .get_active_snapshot(&fixture.game)
            .unwrap()
            .unwrap()
            .snapshot_json,
        before_record
    );
    let cas = CasStore::open(&fixture.library).unwrap();
    assert_eq!(
        fs::read_dir(cas.root().join("objects")).unwrap().count(),
        1,
        "{case}: invalid artifact published"
    );
    let staging = fixture.library.join(".fusion-source/external-staging");
    assert_eq!(
        fs::read_dir(staging).unwrap().count(),
        0,
        "{case}: partial staging survived"
    );
    // A failed attempt must not poison subsequent acquisition or hold writers/locks.
    fixture.document = original;
    fixture.publish();
    fixture
        .service
        .source_refresh(&fixture.game.source_id)
        .await
        .unwrap();
    fixture
        .service
        .game_update(&fixture.game, InstallRequest::default())
        .await
        .unwrap();
    assert_eq!(
        fs::read(fixture.active().workspace_root.join("userdata/save.dat")).unwrap(),
        b"latest private progress"
    );
}

macro_rules! rejected_update_test {
    ($name:ident, $case:literal) => {
        #[tokio::test]
        async fn $name() {
            rejected_update_preserves_installation($case).await;
        }
    };
}
rejected_update_test!(external_rejects_http_without_installation_mutation, "http");
rejected_update_test!(
    external_rejects_url_credentials_without_installation_mutation,
    "credentials"
);
rejected_update_test!(
    external_rejects_wrong_transport_without_installation_mutation,
    "transport"
);
rejected_update_test!(
    external_rejects_wrong_inventory_without_installation_mutation,
    "inventory"
);
rejected_update_test!(
    external_rejects_interrupted_content_length_without_installation_mutation,
    "truncated"
);
rejected_update_test!(
    external_rejects_short_eof_body_without_installation_mutation,
    "eof"
);
rejected_update_test!(
    external_rejects_declared_oversize_without_installation_mutation,
    "oversized"
);
rejected_update_test!(
    external_rejects_https_downgrade_without_installation_mutation,
    "redirect"
);
rejected_update_test!(
    external_rejects_missing_exact_source_root_without_installation_mutation,
    "source-root"
);

async fn rejected_bounded_acquisition(
    mode: &str,
    max_bytes: u64,
    zip_limits: super::super::io::ZipNormalizationLimits,
) {
    use crate::fusion_source::adapters::{Cancellation, TransferLimits};
    let fixture = Fixture::new();
    fixture.connect().await;
    fixture
        .service
        .game_install(&fixture.game, InstallRequest::default())
        .await
        .unwrap();
    let active = fixture.active();
    let before = workspace_bytes(&active.workspace_root);
    let mut component: crate::fusion_source::program_schema::ProgramComponent =
        serde_json::from_value(
            fixture.document["creator"]["program_descriptor"]["components"][0].clone(),
        )
        .unwrap();
    if let crate::fusion_source::creator_schema::ComponentOrigin::External { url, .. } =
        &mut component.origin
    {
        *url = fixture.server.url(mode);
    }
    let error = crate::fusion_source::external::acquire(
        &fixture.library,
        &component,
        Arc::new(SecureHttpsTransport),
        &TransferLimits {
            max_bytes,
            timeout: std::time::Duration::from_secs(10),
        },
        zip_limits,
        &Cancellation::default(),
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error.code,
            crate::fusion_source::ErrorCode::TransferLimitExceeded
                | crate::fusion_source::ErrorCode::InvalidPath
        ),
        "{error:?}"
    );
    assert_eq!(workspace_bytes(&active.workspace_root), before);
    assert_eq!(
        fs::read_dir(fixture.library.join(".fusion-source/external-staging"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(
            CasStore::open(&fixture.library)
                .unwrap()
                .root()
                .join("objects")
        )
        .unwrap()
        .count(),
        1
    );
}

#[tokio::test]
async fn external_streamed_bytes_are_bounded_without_content_length() {
    rejected_bounded_acquisition("chunked-limit", 8192, Default::default()).await;
}

#[tokio::test]
async fn external_extracted_bytes_are_bounded_by_shared_normalizer() {
    rejected_bounded_acquisition(
        "payload",
        1024 * 1024,
        super::super::io::ZipNormalizationLimits {
            max_uncompressed_bytes: 1,
            ..Default::default()
        },
    )
    .await;
}

#[tokio::test]
async fn external_archive_entry_count_is_bounded_by_shared_normalizer() {
    rejected_bounded_acquisition(
        "payload",
        1024 * 1024,
        super::super::io::ZipNormalizationLimits {
            max_entries: 1,
            ..Default::default()
        },
    )
    .await;
}

/// Signal only after the real HTTPS writer has closed and returned its receipt.
/// On the single async thread, acquisition cannot yield between this signal and
/// awaiting its queued blocking verification worker.
struct ReceiptSignallingHttps {
    completed: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[async_trait::async_trait]
impl crate::fusion_source::adapters::ArtifactTransport for ReceiptSignallingHttps {
    async fn fetch(
        &self,
        url: &str,
        destination: &std::path::Path,
        limits: &crate::fusion_source::adapters::TransferLimits,
        cancel: &crate::fusion_source::adapters::Cancellation,
    ) -> Result<crate::fusion_source::adapters::TransferReceipt, crate::fusion_source::FusionError>
    {
        self.fetch_with_progress(url, destination, limits, cancel, None)
            .await
    }

    async fn fetch_with_progress(
        &self,
        url: &str,
        destination: &std::path::Path,
        limits: &crate::fusion_source::adapters::TransferLimits,
        cancel: &crate::fusion_source::adapters::Cancellation,
        progress: Option<Arc<dyn crate::fusion_source::adapters::TransferProgressSink>>,
    ) -> Result<crate::fusion_source::adapters::TransferReceipt, crate::fusion_source::FusionError>
    {
        let receipt = SecureHttpsTransport
            .fetch_with_progress(url, destination, limits, cancel, progress)
            .await?;
        self.completed
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        Ok(receipt)
    }
}

#[test]
fn external_aborting_queued_transport_verification_cleans_entire_acquisition() {
    use crate::fusion_source::adapters::{Cancellation, TransferLimits};
    use std::time::Duration;

    // RAII release and bounded waits also keep a failed regression from hanging
    // runtime teardown while its only blocking thread is deliberately occupied.
    struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let fixture = Fixture::new();
        fixture.connect().await;
        let component: crate::fusion_source::program_schema::ProgramComponent =
            serde_json::from_value(
                fixture.document["creator"]["program_descriptor"]["components"][0].clone(),
            )
            .unwrap();
        let (release, blocked) = std::sync::mpsc::channel();
        let release = ReleaseOnDrop(Some(release));
        let (started, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            blocked
                .recv_timeout(Duration::from_secs(20))
                .expect("bounded blocker release");
        });
        tokio::time::timeout(Duration::from_secs(10), ready)
            .await
            .unwrap()
            .unwrap();
        let (completed, receipt) = tokio::sync::oneshot::channel();
        let transport = Arc::new(ReceiptSignallingHttps {
            completed: Mutex::new(Some(completed)),
        });
        let library = fixture.library.clone();
        let acquisition = tokio::spawn(async move {
            crate::fusion_source::external::acquire(
                &library,
                &component,
                transport,
                &TransferLimits {
                    max_bytes: 1024 * 1024,
                    timeout: Duration::from_secs(10),
                },
                Default::default(),
                &Cancellation::default(),
                None,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), receipt)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !acquisition.is_finished(),
            "verification must still be queued"
        );
        let staging = fixture.library.join(".fusion-source/external-staging");
        assert_eq!(
            fs::read_dir(&staging).unwrap().count(),
            1,
            "acquisition owns staging"
        );
        acquisition.abort();
        let error = tokio::time::timeout(Duration::from_secs(10), acquisition)
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.is_cancelled());
        drop(release);
        tokio::time::timeout(Duration::from_secs(10), blocker)
            .await
            .unwrap()
            .unwrap();
        // With exactly one blocking thread, this sentinel runs only after the
        // already queued verification closure and all of its guards are dropped.
        tokio::time::timeout(Duration::from_secs(10), tokio::task::spawn_blocking(|| ()))
            .await
            .unwrap()
            .unwrap();
        assert!(fixture
            .storage
            .get_active_snapshot(&fixture.game)
            .unwrap()
            .is_none());
        assert!(fixture
            .storage
            .list_snapshots(&fixture.game)
            .unwrap()
            .is_empty());
        let cas = CasStore::open(&fixture.library).unwrap();
        assert_eq!(
            fs::read_dir(cas.root().join("objects")).unwrap().count(),
            0,
            "aborted acquisition must not import into library CAS"
        );
        assert_eq!(
            fs::read_dir(&staging).unwrap().count(),
            0,
            "aborted transport verification must remove the outer acquire-* directory"
        );
    });
}

#[tokio::test]
async fn external_wrong_inventory_on_clean_cache_never_publishes_library_artifact() {
    let mut fixture = Fixture::new();
    fixture.document["creator"]["program_descriptor"]["components"][0]["origin"]
        ["inventory_blake3"] = "b".repeat(64).into();
    fixture.publish();
    fixture.connect().await;
    assert_eq!(
        fixture
            .service
            .game_install(&fixture.game, InstallRequest::default())
            .await
            .unwrap_err()
            .code,
        crate::fusion_source::ErrorCode::IntegrityViolation
    );
    assert!(fixture
        .storage
        .get_active_snapshot(&fixture.game)
        .unwrap()
        .is_none());
    assert!(fixture
        .storage
        .list_snapshots(&fixture.game)
        .unwrap()
        .is_empty());
    let cas = CasStore::open(&fixture.library).unwrap();
    assert_eq!(fs::read_dir(cas.root().join("objects")).unwrap().count(), 0);
    assert_eq!(
        fs::read_dir(fixture.library.join(".fusion-source/external-staging"))
            .unwrap()
            .count(),
        0
    );
}
