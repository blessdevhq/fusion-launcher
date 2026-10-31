use super::*;
use crate::fusion_source::canonical::QualifiedGameId;
use crate::fusion_source::dto::{GameOptions, InstallRequest};
use crate::fusion_source::packer::PackerEngine;
use crate::fusion_source::service::{FusionHostAdapters, FusionStorage};
use crate::storage::RepositoryStore;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

fn environment(root: &Path) -> ServiceEnvironment {
    ServiceEnvironment {
        appdata: root.join("appdata"),
        local_appdata: root.join("localappdata"),
        display_width: 1920,
        display_height: 1080,
        refresh_rate: 60,
    }
}

fn build(root: &Path, version: &str, source_id: &str) -> PathBuf {
    let input = root.join("input");
    std::fs::create_dir_all(input.join("user/config")).unwrap();
    std::fs::create_dir_all(input.join("user/nand")).unwrap();
    if !input.join("eden.exe").exists() {
        std::fs::copy(std::env::current_exe().unwrap(), input.join("eden.exe")).unwrap();
    }
    let mut critical = vec![serde_json::json!({"path":"eden.exe","role":"main_executable"})];
    let activation = PathBuf::from(format!(
        "{}.manifest",
        std::env::current_exe().unwrap().display()
    ));
    if activation.is_file() {
        std::fs::copy(activation, input.join("eden.exe.manifest")).unwrap();
        critical.push(serde_json::json!({"path":"eden.exe.manifest","role":"core_library"}));
    }
    std::fs::write(input.join("user/config/qt-config.ini"), "[Controls]\nmouse_panning_x_sensitivity=68\n[Data%20Storage]\nnand_directory=F:/old/user/nand\nnand_directory\\default=false\n").unwrap();
    std::fs::write(input.join("user/nand/progress.dat"), b"initial save").unwrap();
    let addon = root.join("addon");
    std::fs::create_dir_all(&addon).unwrap();
    std::fs::write(addon.join("graphics.txt"), b"optional graphics").unwrap();
    let recipe = root.join(format!("recipe-{source_id}-{version}.json"));
    std::fs::write(&recipe, serde_json::to_vec(&serde_json::json!({
        "source_info":{"id":source_id,"name":"Local desktop fixture"},
        "base_url":format!("https://offline.fusion.invalid/{source_id}/"),
        "games":[{"id":"demo","title":"Desktop fixture","version":version,
            "layers":[
                {"id":"runtime","kind":"runtime","local_path":"input","mount_path":"runtime","critical_files":critical},
                {"id":"graphics","kind":"mod","local_path":"addon","mount_path":"mods","optional":true,"default_enabled":true}
            ],
            "runtime":{"layer_id":"runtime","executable":"runtime/eden.exe","args":["--exact","local_sources::tests::desktop_native_fixture","--nocapture"],"env":{"FUSION_LOCAL_DESKTOP_FIXTURE":"1"}},
            "persistence":{"saves_paths":["workspace/runtime/user"]},
            "config_templates":[{"id":"quality","path":"settings/quality.ini","format":"ini","template":format!("[graphics]\nrelease={version}\n")}]
        }]
    })).unwrap()).unwrap();
    let output = root.join(format!("source-{source_id}-{version}"));
    PackerEngine::new().build_file(&recipe, &output).unwrap();
    output
}

/// Real supervised child, confined to its own workspace/persistence fixture.
#[test]
fn desktop_native_fixture() {
    if std::env::var("FUSION_LOCAL_DESKTOP_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    let workspace = std::env::current_dir().unwrap();
    let config =
        std::fs::read_to_string(workspace.join("runtime/user/config/qt-config.ini")).unwrap();
    assert!(
        config.contains("Paths\\gamedirs\\size=1"),
        "host preparation ran before the child"
    );
    assert!(config.contains("mouse_panning_x_sensitivity=68"));
    assert!(!config.contains("F:/old"));
    std::fs::write(
        workspace.join("runtime/user/nand/progress.dat"),
        b"saved by supervised child",
    )
    .unwrap();
}

#[tokio::test]
async fn desktop_local_import_restart_launch_update_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let source_one = build(root.path(), "1", "owned");
    let library = root.path().join("library");
    let database = root.path().join("desktop.sqlite");
    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
        RepositoryStore::open(&database).unwrap(),
    ))));
    let candidate = LocalSourceCandidate::open(
        &source_one,
        &library,
        storage.clone(),
        environment(root.path()),
    )
    .unwrap();
    let preview = candidate.preview().await.unwrap();
    assert_eq!(preview.source.game_count, 1);
    assert!(
        storage.list_sources().unwrap().is_empty(),
        "preview does not connect or install"
    );
    let connected = candidate.connect(&storage, "owned", true).await.unwrap();
    assert!(connected.prepare_eden);
    assert!(connected.local_directory.is_some());
    drop(storage);
    // A new desktop database connection/transport recovers host-owned authority.
    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
        RepositoryStore::open(&database).unwrap(),
    ))));
    assert_eq!(
        storage.local_source("owned").unwrap().unwrap().directory,
        source_one.to_string_lossy()
    );
    let service = FusionService::open(
        &library,
        storage.clone(),
        Arc::new(DesktopSourceTransport::new(storage.clone())),
        Arc::new(NoProgress),
        environment(root.path()),
    )
    .unwrap()
    .with_host_adapters(FusionHostAdapters {
        launch_preparation: Some(Arc::new(LocalProfilePreparation(storage.clone()))),
        ..Default::default()
    })
    .unwrap();
    let game = QualifiedGameId {
        source_id: "owned".into(),
        game_id: "demo".into(),
    };
    service
        .game_install(&game, InstallRequest::default())
        .await
        .unwrap();
    let state = service.game_get_state(&game).await.unwrap();
    assert!(!state.ready);
    for file in &state.executables {
        service
            .game_trust_executable(&game, &file.path, &file.blake3)
            .await
            .unwrap();
    }
    service.game_launch(&game).await.unwrap();
    let after_launch = service.game_get_state(&game).await.unwrap();
    if let Some(report) = after_launch.crash_reports.first() {
        panic!(
            "Fixture child failed: {:?}",
            service.crash_preview(&report.report_id).unwrap()
        );
    }
    let save = library.join(
        ".fusion-source/persistence/owned/demo/data/workspace/runtime/user/nand/progress.dat",
    );
    assert_eq!(std::fs::read(&save).unwrap(), b"saved by supervised child");
    service
        .game_set_options(
            &game,
            GameOptions {
                enabled_addons: Some(Default::default()),
                runtime_channel: None,
                ..GameOptions::default()
            },
        )
        .await
        .unwrap();
    service
        .game_update(&game, InstallRequest::default())
        .await
        .unwrap();
    let previous = service.game_get_state(&game).await.unwrap();
    let source_two = build(root.path(), "2", "owned");
    // Replacing an input folder is refused while a DB operation owns the source.
    let operation = storage.begin_operation(&game, "update").unwrap();
    let candidate = LocalSourceCandidate::open(
        &source_two,
        &library,
        storage.clone(),
        environment(root.path()),
    )
    .unwrap();
    assert!(matches!(
        candidate.connect(&storage, "owned", true).await,
        Err(FusionError {
            code: ErrorCode::OperationInProgress,
            ..
        })
    ));
    assert_eq!(
        storage.local_source("owned").unwrap().unwrap().directory,
        source_one.to_string_lossy()
    );
    storage
        .finish_operation(&operation.id, "cancelled")
        .unwrap();
    LocalSourceCandidate::open(
        &source_two,
        &library,
        storage.clone(),
        environment(root.path()),
    )
    .unwrap()
    .connect(&storage, "owned", true)
    .await
    .unwrap();
    assert!(
        service
            .game_get_state(&game)
            .await
            .unwrap()
            .update_available
    );
    service
        .game_update(&game, InstallRequest::default())
        .await
        .unwrap();
    let state = service.game_get_state(&game).await.unwrap();
    assert_eq!(state.active_version.as_deref(), Some("2"));
    let assembler =
        crate::fusion_source::workspace::WorkspaceAssembler::open(&library, &game).unwrap();
    {
        let active = assembler.active().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(active.snapshot.workspace_root.join("settings/quality.ini"))
                .unwrap(),
            "[graphics]\nrelease=2\n"
        );
        assert!(!active
            .snapshot
            .workspace_root
            .join("mods/graphics.txt")
            .exists());
    }
    service.game_rollback(&game).await.unwrap();
    assert_eq!(
        service
            .game_get_state(&game)
            .await
            .unwrap()
            .active_snapshot_id,
        previous.active_snapshot_id
    );
    assert_eq!(std::fs::read(&save).unwrap(), b"saved by supervised child");
    // Wrong-folder imports never replace a previously connected source.
    let other = build(root.path(), "1", "other");
    assert!(matches!(
        LocalSourceCandidate::open(&other, &library, storage.clone(), environment(root.path()))
            .unwrap()
            .connect(&storage, "owned", false)
            .await,
        Err(FusionError {
            code: ErrorCode::SnapshotConflict,
            ..
        })
    ));
    assert_eq!(
        storage.local_source("owned").unwrap().unwrap().directory,
        source_two.to_string_lossy()
    );
}

struct CountingNetwork(Arc<AtomicUsize>);
#[async_trait]
impl ArtifactTransport for CountingNetwork {
    async fn fetch(
        &self,
        _: &str,
        _: &Path,
        _: &TransferLimits,
        _: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(FusionError::new(
            ErrorCode::Network,
            "test network disabled",
        ))
    }
}

#[tokio::test]
async fn reserved_local_namespace_never_falls_back_to_network() {
    let root = tempfile::tempdir().unwrap();
    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
        RepositoryStore::open(&root.path().join("desktop.sqlite")).unwrap(),
    ))));
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = DesktopSourceTransport {
        storage,
        network: Arc::new(CountingNetwork(calls.clone())),
    };
    for url in [
        "https://offline.fusion.invalid/unknown/source-index.json",
        "https://OFFLINE.FUSION.INVALID/unknown/source-index.json",
        "https://offline.fusion.invalid./unknown/source-index.json",
        "https://offline.fusion.invalid/unknown/%2e%2e/secret",
        "https://offline.fusion.invalid/unknown/source-index.json?x=1",
    ] {
        assert!(transport
            .fetch(
                url,
                &root.path().join("copy"),
                &TransferLimits::json(),
                &Cancellation::default()
            )
            .await
            .is_err());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(transport
        .fetch(
            "https://example.test/source-index.json",
            &root.path().join("copy"),
            &TransferLimits::json(),
            &Cancellation::default()
        )
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
