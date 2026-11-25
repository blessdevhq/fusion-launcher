//! Windows SDL controller binding, release barrier, and Eden ParamPackage end-to-end tests.
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::fusion_creator::bridge::production_runner;
use crate::fusion_creator::input::{
    InputBindingsSaveRequest, InputProvider, InputSessionRequest, InputSessionStartRequest,
    InputSessionStatus,
};
use crate::fusion_creator::jobs::JobRunner;
use crate::fusion_creator::{
    project_service, project_storage, CreatorJobKind, CreatorJobStartRequest, CreatorJobStatus,
    CreatorPrivateBindings, CreatorReportKind, CreatorReportStatus,
};
use crate::fusion_source::canonical::QualifiedGameId;
use crate::fusion_source::creator_schema::LogicalBinding;
use crate::fusion_source::dto::{GameOptions, InstallRequest};
use crate::fusion_source::fs_safety;
use crate::fusion_source::packer::{PackerEngine, PackerRecipe};
use crate::fusion_source::program_schema::{CreatorProgramProject, ProgramDescriptor};
use crate::fusion_source::service::{FusionService, NoProgress, ServiceEnvironment};
use crate::fusion_source::signatures;
use crate::fusion_source::workspace::WorkspaceAssembler;
use crate::input::service::InputService;
use crate::local_sources::{DesktopSourceTransport, LocalSourceCandidate};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};

const SDL_SPAWN_HELPER: &str =
    "fusion_creator::declarative::runtime_sdl_tests::sdl_spawn_marker_helper";
const SOURCE_ID: &str = "eden-sdl-source";
const GAME_ID: &str = "eden-sdl-demo";
const HELPER_ENV: &str = "FUSION_EDEN_SDL_MARKER";

#[test]
fn sdl_spawn_marker_helper() {
    let Some(dir) = std::env::var_os(HELPER_ENV) else {
        return;
    };
    let marker = PathBuf::from(dir).join("spawned.txt");
    fs_safety::atomic_write(&marker, b"spawned").unwrap();
}

struct EdenServiceFixture {
    _temp: tempfile::TempDir,
    game: QualifiedGameId,
    library: PathBuf,
    service: FusionService,
    input: Arc<InputService>,
}

fn test_environment(root: &Path) -> ServiceEnvironment {
    ServiceEnvironment {
        appdata: root.join("appdata"),
        local_appdata: root.join("local_appdata"),
        display_width: 1280,
        display_height: 720,
        refresh_rate: 60,
    }
}

fn wait_for_sdl_lease_release() {
    let deadline = Instant::now() + Duration::from_secs(3);
    while crate::input::sdl_leased() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for SDL lease release"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn eden_descriptor_value() -> serde_json::Value {
    json!({
        "document_type": "program_descriptor",
        "schema_version": "1.0",
        "identity": {
            "id": GAME_ID,
            "title": "Eden SDL Harness",
            "version": "0.2.1",
            "os": "windows",
            "architecture": "x86_64"
        },
        "executor": {"id": "fusion.declarative", "version": "1.0"},
        "components": [{
            "id": "eden-bin",
            "layer_id": "runtime",
            "origin": {"kind": "managed", "artifact_ref": "eden-bin", "redistribution_allowed": true},
            "source_root": "",
            "mount_path": "eden"
        }],
        "required_files": [{"path": "eden/eden.exe", "role": "main_executable"}],
        "projection_rules": [],
        "directories": ["eden/user/config", "userdata"],
        "entrypoint": {
            "executable": "eden/eden.exe",
            "working_directory": "eden",
            "arguments": [
                {"kind": "literal", "value": "--exact"},
                {"kind": "literal", "value": SDL_SPAWN_HELPER},
                {"kind": "literal", "value": "--nocapture"}
            ],
            "environment_paths": [
                {"name": HELPER_ENV, "path": "userdata"}
            ]
        },
        "configs": [{
            "id": "eden-main",
            "path": "eden/user/config/qt-config.ini",
            "format": "ini",
            "seed": {"kind": "empty_ini"},
            "preserve_unmanaged": true,
            "keys": [{
                "id": "backend",
                "address": {"kind": "ini", "section": "Renderer", "key": "backend"},
                "scalar_type": "integer",
                "allowed_values": [0, 1]
            }]
        }],
        "persistence": [],
        "build_profiles": [{
            "id": "vulkan",
            "title": "Vulkan",
            "layer_ids": ["runtime"],
            "controls_profile_ids": ["sdl-p0", "kb-p0"],
            "settings": [{"config_id": "eden-main", "key_id": "backend", "value": 1}]
        }],
        "controls_profiles": [
            {
                "id": "sdl-p0",
                "title": "SDL Player 0",
                "kind": "eden_param_package_v1",
                "device": "sdl_gamecontroller",
                "config_id": "eden-main",
                "bindings": {
                    "a": {"kind": "controller_button", "button": "b"},
                    "b": {"kind": "controller_button", "button": "a"},
                    "lstick_up": {"kind": "controller_axis", "axis": "lefty", "direction": "negative"},
                    "lstick_down": {"kind": "controller_axis", "axis": "lefty", "direction": "positive"},
                    "lstick_left": {"kind": "controller_axis", "axis": "leftx", "direction": "negative"},
                    "lstick_right": {"kind": "controller_axis", "axis": "leftx", "direction": "positive"}
                }
            },
            {
                "id": "kb-p0",
                "title": "Keyboard Player 0",
                "kind": "eden_param_package_v1",
                "device": "keyboard_mouse",
                "config_id": "eden-main",
                "bindings": {
                    "a": {"kind": "keyboard", "code": "KeyX"}
                }
            }
        ]
    })
}

async fn setup_eden_sdl_fixture() -> EdenServiceFixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let author = root.join("author");
    let eden_dir = author.join("eden-dir");
    let publication = root.join("publication");
    let library = root.join("library");
    let db_path = root.join("player.sqlite");
    fs::create_dir_all(&eden_dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    fs::copy(&exe, eden_dir.join("eden.exe")).unwrap();
    let manifest_sidecar = PathBuf::from(format!("{}.manifest", exe.display()));
    if manifest_sidecar.is_file() {
        fs::copy(manifest_sidecar, eden_dir.join("eden.exe.manifest")).unwrap();
    }
    let key_path = author.join("author.pem");
    signatures::keygen(&key_path).unwrap();

    let descriptor_val = eden_descriptor_value();
    let program: ProgramDescriptor = serde_json::from_value(descriptor_val.clone()).unwrap();
    let layers = vec![json!({
        "id": "runtime",
        "kind": "runtime",
        "local_path": eden_dir,
        "archive": "zip"
    })];
    let recipe: PackerRecipe = serde_json::from_value(json!({
        "source_info": {"id": SOURCE_ID, "name": "Eden SDL Test Source"},
        "games": [{
            "id": GAME_ID,
            "title": "Eden SDL Harness",
            "version": "0.2.1",
            "layers": layers,
            "program_descriptor": descriptor_val
        }]
    }))
    .unwrap();
    PackerEngine::new()
        .build_with_signing_key(&recipe, root, &publication, Some(&key_path))
        .unwrap();

    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
        RepositoryStore::open(&db_path).unwrap(),
    ))));
    let candidate = LocalSourceCandidate::open(
        &publication,
        &library,
        storage.clone(),
        test_environment(root),
    )
    .unwrap();
    assert!(candidate.preview().await.unwrap().source.signature_verified);
    candidate.connect(&storage, SOURCE_ID, false).await.unwrap();

    let game = QualifiedGameId {
        source_id: SOURCE_ID.into(),
        game_id: GAME_ID.into(),
    };
    let approval = super::runtime::EnvironmentApproval::for_descriptor(
        &program,
        BTreeSet::from([HELPER_ENV.into()]),
    )
    .unwrap();
    let input = Arc::new(InputService::new(root.join("fusion-input-private"), |_| {}));
    let service = FusionService::open(
        &library,
        storage.clone(),
        Arc::new(DesktopSourceTransport::new(storage)),
        Arc::new(NoProgress),
        test_environment(root),
    )
    .unwrap()
    .with_program_environment_approval(game.clone(), approval)
    .unwrap()
    .with_input_service(Arc::clone(&input))
    .unwrap();

    EdenServiceFixture {
        _temp: temp,
        game,
        library,
        service,
        input,
    }
}

fn save_virtual_sdl_bindings(input: &InputService) -> String {
    let device_id = input.fixture_attach().unwrap();
    let receipt = input.start(InputSessionStartRequest { device_id }).unwrap();
    input
        .save_bindings(InputBindingsSaveRequest {
            session_id: receipt.session_id.clone(),
            controls_profile_id: "sdl-p0".into(),
            bindings: BTreeMap::from([
                (
                    "a".into(),
                    LogicalBinding::ControllerButton { button: "b".into() },
                ),
                (
                    "b".into(),
                    LogicalBinding::ControllerButton { button: "a".into() },
                ),
                (
                    "lstick_up".into(),
                    LogicalBinding::ControllerAxis {
                        axis: "lefty".into(),
                        direction: crate::fusion_source::creator_schema::AxisDirection::Negative,
                    },
                ),
                (
                    "lstick_down".into(),
                    LogicalBinding::ControllerAxis {
                        axis: "lefty".into(),
                        direction: crate::fusion_source::creator_schema::AxisDirection::Positive,
                    },
                ),
                (
                    "lstick_left".into(),
                    LogicalBinding::ControllerAxis {
                        axis: "leftx".into(),
                        direction: crate::fusion_source::creator_schema::AxisDirection::Negative,
                    },
                ),
                (
                    "lstick_right".into(),
                    LogicalBinding::ControllerAxis {
                        axis: "leftx".into(),
                        direction: crate::fusion_source::creator_schema::AxisDirection::Positive,
                    },
                ),
            ]),
            deadzones: BTreeMap::from([("leftx".into(), 0.2)]),
            raw_bindings: BTreeMap::new(),
        })
        .unwrap();
    receipt.session_id
}

async fn install_and_trust_eden_sdl(fixture: &EdenServiceFixture) -> PathBuf {
    fixture
        .service
        .game_set_options(
            &fixture.game,
            GameOptions {
                enabled_addons: None,
                runtime_channel: None,
                build_profile_id: Some("vulkan".into()),
                controls_profile_id: Some("sdl-p0".into()),
            },
        )
        .await
        .unwrap();
    fixture
        .service
        .game_install(&fixture.game, InstallRequest::default())
        .await
        .unwrap();
    let state = fixture.service.game_get_state(&fixture.game).await.unwrap();
    assert_eq!(state.build_profiles.len(), 1);
    assert_eq!(state.controls_profiles.len(), 2);
    fixture
        .service
        .game_trust_executables(
            &fixture.game,
            state
                .executables
                .iter()
                .map(|exe| (exe.path.clone(), exe.blake3.clone()))
                .collect(),
        )
        .await
        .unwrap();
    WorkspaceAssembler::open(&fixture.library, &fixture.game)
        .unwrap()
        .active()
        .unwrap()
        .unwrap()
        .snapshot
        .workspace_root
}

// TEST_OWNER serializes process-global SDL context ownership across parallel test threads.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn eden_profile_sdl_release_barrier_real_spawn_and_rediscovery() {
    let fixture = setup_eden_sdl_fixture().await;
    let _owner = crate::input::tests::TEST_OWNER.lock().unwrap();
    let session_id = save_virtual_sdl_bindings(&fixture.input);
    let workspace = install_and_trust_eden_sdl(&fixture).await;

    let ini = fs::read_to_string(workspace.join("eden/user/config/qt-config.ini")).unwrap();
    assert!(ini.contains("[Renderer]"));
    assert!(ini.contains("backend=1"));
    assert!(ini.contains("[Controls]"));
    assert!(ini.contains("player_0_connected=true"));
    assert!(ini.contains("enable_raw_input=false"));
    assert!(ini.contains("enable_joycon_driver=true"));
    assert!(ini.contains("enable_procon_driver=false"));
    assert!(ini.contains("player_0_button_a=\"button:1,engine:sdl,guid:"));
    assert!(ini.contains("player_0_lstick=\""));
    assert!(ini.contains("deadzone:0.200000"));

    let marker = workspace.join("userdata/spawned.txt");
    assert!(!marker.exists());

    fixture.service.game_launch(&fixture.game).await.unwrap();
    assert!(
        marker.is_file(),
        "expected real child process to write marker"
    );
    let stopped = fixture
        .input
        .state(InputSessionRequest {
            session_id: session_id.clone(),
        })
        .unwrap();
    assert_eq!(stopped.status, InputSessionStatus::Stopped);

    // Rediscover cleanly on a fresh SDL worker after child exit.
    let _ = fixture.input.devices().unwrap();
    let reattached = fixture.input.fixture_attach().unwrap();
    let next = fixture
        .input
        .start(InputSessionStartRequest {
            device_id: reattached,
        })
        .unwrap();
    assert_ne!(next.session_id, session_id);
    assert!(
        fixture
            .input
            .stop(InputSessionRequest {
                session_id: next.session_id,
            })
            .unwrap()
            .handles_closed
    );
    wait_for_sdl_lease_release();
}

// TEST_OWNER serializes process-global SDL context ownership across parallel test threads.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn failed_sdl_release_ack_blocks_real_spawn() {
    let fixture = setup_eden_sdl_fixture().await;
    let _owner = crate::input::tests::TEST_OWNER.lock().unwrap();
    let _session_id = save_virtual_sdl_bindings(&fixture.input);
    let workspace = install_and_trust_eden_sdl(&fixture).await;

    fixture
        .input
        .fixture_release_fault(Duration::ZERO, true)
        .unwrap();
    let marker = workspace.join("userdata/spawned.txt");
    let error = fixture
        .service
        .game_launch(&fixture.game)
        .await
        .unwrap_err();
    assert_eq!(
        error.code,
        crate::fusion_source::ErrorCode::ActionUnavailable
    );
    assert!(
        !marker.exists(),
        "real child process must never spawn when SDL release fails"
    );
    wait_for_sdl_lease_release();
}

fn run_creator_job(
    runner: &JobRunner,
    kind: CreatorJobKind,
    request: CreatorJobStartRequest,
) -> crate::fusion_creator::CreatorJob {
    let deadline = Instant::now() + Duration::from_secs(45);
    let receipt = loop {
        match runner.start(kind.clone(), request.clone()) {
            Ok(receipt) => break receipt,
            Err(err)
                if err.code == crate::fusion_source::ErrorCode::OperationInProgress
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("failed starting creator job: {err:?}"),
        }
    };
    loop {
        let job = runner.get(&receipt.job_id).unwrap();
        if matches!(
            job.progress.status,
            CreatorJobStatus::Completed
                | CreatorJobStatus::Failed
                | CreatorJobStatus::Cancelled
                | CreatorJobStatus::Interrupted
        ) {
            return job;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {job:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn creator_build_and_verify_support_eden_keyboard_profile_end_to_end() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let runtime_dir = root.join("author-eden");
    fs::create_dir(&runtime_dir).unwrap();
    fs::write(runtime_dir.join("eden.exe"), b"MZ-synthetic-eden-binary").unwrap();
    let key_path = root.join("author-signing.pem");
    signatures::keygen(&key_path).unwrap();

    let project_val = json!({
        "schema_version": "1.1",
        "project_id": "eden-kb-proj",
        "revision": 1,
        "title": "Eden Keyboard Creator Project",
        "source_id": "eden-kb-source",
        "game_id": "eden-kb-game",
        "game_version": "0.2.1",
        "program_descriptor": {
            "document_type": "program_descriptor",
            "schema_version": "1.0",
            "identity": {
                "id": "eden-kb-game",
                "title": "Eden Keyboard Creator Project",
                "version": "0.2.1",
                "os": "windows",
                "architecture": "x86_64"
            },
            "executor": {"id": "fusion.declarative", "version": "1.0"},
            "components": [{
                "id": "eden-bin",
                "layer_id": "runtime",
                "origin": {"kind": "managed", "artifact_ref": "eden-bin", "redistribution_allowed": true},
                "source_root": "",
                "mount_path": "eden"
            }],
            "required_files": [{"path": "eden/eden.exe", "role": "main_executable"}],
            "projection_rules": [],
            "directories": ["eden/user/config"],
            "entrypoint": {
                "executable": "eden/eden.exe",
                "working_directory": "eden",
                "arguments": [],
                "environment_paths": []
            },
            "configs": [{
                "id": "eden-main",
                "path": "eden/user/config/qt-config.ini",
                "format": "ini",
                "seed": {"kind": "empty_ini"},
                "preserve_unmanaged": true,
                "keys": [{
                    "id": "backend",
                    "address": {"kind": "ini", "section": "Renderer", "key": "backend"},
                    "scalar_type": "integer",
                    "allowed_values": [0, 1]
                }]
            }],
            "persistence": [],
            "build_profiles": [{
                "id": "vulkan",
                "title": "Vulkan",
                "layer_ids": ["runtime"],
                "controls_profile_ids": ["kb-p0"],
                "settings": [{"config_id": "eden-main", "key_id": "backend", "value": 1}]
            }],
            "controls_profiles": [{
                "id": "kb-p0",
                "title": "Keyboard P0",
                "kind": "eden_param_package_v1",
                "device": "keyboard_mouse",
                "config_id": "eden-main",
                "bindings": {
                    "a": {"kind": "keyboard", "code": "KeyX"},
                    "lstick_up": {"kind": "keyboard", "code": "KeyW"}
                }
            }]
        }
    });
    let _parsed: CreatorProgramProject = serde_json::from_value(project_val.clone()).unwrap();

    let mut store = RepositoryStore::open(&root.join("creator.sqlite")).unwrap();
    project_storage::save_project_bytes(
        &mut store,
        &serde_json::to_vec(&project_val).unwrap(),
        None,
    )
    .unwrap();
    project_storage::save_private_bindings(
        &store,
        1,
        None,
        &CreatorPrivateBindings {
            project_id: "eden-kb-proj".into(),
            component_paths: [("eden-bin".into(), runtime_dir.to_string_lossy().into())].into(),
            signing_key_ref: Some(key_path.to_string_lossy().into()),
        },
    )
    .unwrap();
    let store = Arc::new(Mutex::new(store));
    let runner =
        production_runner(Arc::clone(&store), &root.join("creator"), Arc::new(|_| {})).unwrap();

    let req = |release_id: Option<String>, target: Option<&Path>| CreatorJobStartRequest {
        project_id: "eden-kb-proj".into(),
        project_revision: 1,
        build_profile_ids: vec!["vulkan".into()],
        controls_profile_ids: vec!["kb-p0".into()],
        release_id,
        target_directory: target.map(|p| p.to_string_lossy().into()),
        source_url: None,
    };

    let build = run_creator_job(&runner, CreatorJobKind::Build, req(None, None));
    assert_eq!(
        build.progress.status,
        CreatorJobStatus::Completed,
        "{build:?}"
    );
    let releases = project_service::release_list(&store.lock().unwrap(), "eden-kb-proj").unwrap();
    assert_eq!(releases.len(), 1);
    let release_id = releases[0].release_id.clone();

    let verify = run_creator_job(
        &runner,
        CreatorJobKind::Verify,
        req(Some(release_id.clone()), None),
    );
    assert_eq!(
        verify.progress.status,
        CreatorJobStatus::Completed,
        "{verify:?}"
    );
    assert_eq!(verify.report_ids.len(), 2);
    let reports: Vec<_> = {
        let guard = store.lock().unwrap();
        verify
            .report_ids
            .iter()
            .map(|id| project_service::report_get(&guard, id).unwrap())
            .collect()
    };
    assert!(reports.iter().any(|r| {
        r.kind == CreatorReportKind::StructureIntegrity && r.status == CreatorReportStatus::Passed
    }));
    assert!(reports.iter().any(|r| {
        r.kind == CreatorReportKind::CleanInstall && r.status == CreatorReportStatus::Passed
    }));

    let export_dir = root.join("exported-source");
    let export = run_creator_job(
        &runner,
        CreatorJobKind::Export,
        req(Some(release_id), Some(&export_dir)),
    );
    assert_eq!(
        export.progress.status,
        CreatorJobStatus::Completed,
        "{export:?}"
    );
    assert!(export_dir.join("source-index.json").is_file());
    let plain = fs_safety::list_plain_files_checked(&export_dir, &|| Ok(())).unwrap();
    for (rel, path) in plain {
        let bytes = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("author-eden")
                && !text.contains("author-signing")
                && !text.contains("fusion-input-private")
                && !text.contains("-----BEGIN"),
            "export file {rel} leaked private author data"
        );
    }
}
