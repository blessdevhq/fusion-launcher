//! Opt-in live Windows acceptance; no upstream payloads or trust bypasses in Git.
use crate::fusion_source::adapters::SecureHttpsTransport;
use crate::fusion_source::cas::CasStore;
use crate::fusion_source::dto::{GameOptions, InstallRequest, OperationProgress};
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::program_schema::ProgramDescriptor;
use crate::fusion_source::schema::SourceIndex;
use crate::fusion_source::service::{
    FusionService, FusionStorage, ProgressSink, ServiceEnvironment,
};
use crate::fusion_source::test_support::https::HttpsFixture;
use crate::fusion_source::workspace::WorkspaceSnapshot;
use crate::fusion_source::{ErrorCode, QualifiedGameId};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DESCRIPTOR: &str =
    include_str!("../../../../fixtures/creator/programs/supertuxkart-1.5.json");
const CRITICAL: &str =
    include_str!("../../../../fixtures/creator/programs/supertuxkart-1.5-critical-files.json");
const ZIP_BYTES: u64 = 922_430_727;

#[derive(Default)]
struct Evidence(Mutex<Vec<OperationProgress>>);
impl ProgressSink for Evidence {
    fn emit(&self, event: &OperationProgress) {
        self.0.lock().unwrap().push(event.clone());
    }
}

fn publish(root: &Path, server: &HttpsFixture, descriptor: &Value, version: &str) {
    let component = &descriptor["components"][0];
    let critical: Value = serde_json::from_str(CRITICAL).unwrap();
    let manifest = json!({
        "schema_version":"1.1", "id":"portable", "title":"Portable verification", "version":version,
        "creator":{"program_descriptor":descriptor},
        "layers":[{"id":component["layer_id"], "kind":"runtime", "mount_path":component["mount_path"],
          "source":{"kind":"user_provided", "archive":"raw"}, "critical_files":critical}]
    });
    let bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(root.join("game.json"), &bytes).unwrap();
    let mut index: SourceIndex = serde_json::from_value(json!({
        "schema_version":"1.0", "source_info":{"id":"creator-live-cycle", "name":"Creator live cycle"},
        "games":[{"id":"portable", "title":"Portable verification", "version":version,
                  "manifest_url":server.url("game.json"), "manifest_blake3":blake3::hash(&bytes).to_hex().to_string()}]
    })).unwrap();
    crate::fusion_source::signatures::sign_index(
        &mut index,
        &ed25519_dalek::SigningKey::from_bytes(&[53; 32]),
    )
    .unwrap();
    fs::write(root.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
    println!(
        "signed Source release {version}, raw manifest BLAKE3 {}",
        blake3::hash(&bytes).to_hex()
    );
}

fn active(storage: &SqliteFusionStorage, game: &QualifiedGameId) -> WorkspaceSnapshot {
    serde_json::from_str(
        &storage
            .get_active_snapshot(game)
            .unwrap()
            .unwrap()
            .snapshot_json,
    )
    .unwrap()
}

fn options(build: &str) -> GameOptions {
    GameOptions {
        build_profile_id: Some(build.into()),
        controls_profile_id: Some("native".into()),
        ..GameOptions::default()
    }
}

fn config(snapshot: &WorkspaceSnapshot, windowed: bool) {
    let text = fs::read_to_string(
        snapshot
            .workspace_root
            .join("userdata/config-0.10/config.xml"),
    )
    .unwrap();
    assert!(text.contains("<stkconfig version=\"8\""), "{text}");
    assert!(
        text.contains(&format!("fullscreen=\"{}\"", !windowed)),
        "{text}"
    );
    assert!(
        text.contains(&format!("show_fps=\"{}\"", !windowed)),
        "{text}"
    );
    assert!(text.contains("music_on=\"true\""), "{text}");
    println!(
        "verified XML managed settings; build={:?}, controls={:?}",
        snapshot.plan.build_profile_id, snapshot.plan.controls_profile_id
    );
}

fn process(path: &Path) -> Option<u32> {
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "$wanted=[System.IO.Path]::GetFullPath($env:FUSION_OBSERVED_PE); Get-Process -Name ([System.IO.Path]::GetFileNameWithoutExtension($wanted)) -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $wanted } | ForEach-Object { $_.Id }"])
        .env("FUSION_OBSERVED_PE", path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut pids = stdout.lines().flat_map(|line| line.trim().parse::<u32>());
    let pid = pids.next();
    assert!(pids.next().is_none());
    pid
}

#[tokio::test]
#[ignore = "requires public upstream HTTPS, disk space and an interactive Windows graphics session"]
async fn real_supertuxkart_clean_install_observed_launch_update_rollback() {
    let parent = PathBuf::from(
        std::env::var_os("FUSION_STK_TEST_ROOT")
            .expect("set FUSION_STK_TEST_ROOT to a scratch parent"),
    );
    let sandbox = tempfile::Builder::new()
        .prefix("stk-cycle-")
        .tempdir_in(parent)
        .unwrap();
    let root = sandbox.path();
    let library = root.join("library");
    let workspace = root.join("workspace");
    let published = root.join("source");
    fs::create_dir(&published).unwrap();
    fs::create_dir(&workspace).unwrap();
    let db = root.join("player.sqlite");
    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
        RepositoryStore::open(&db).unwrap(),
    ))));
    let evidence = Arc::new(Evidence::default());
    let service = FusionService::open(
        &library,
        storage.clone(),
        Arc::new(SecureHttpsTransport),
        evidence.clone(),
        ServiceEnvironment {
            appdata: root.join("appdata"),
            local_appdata: root.join("local-appdata"),
            display_width: 1280,
            display_height: 720,
            refresh_rate: 60,
        },
    )
    .unwrap();
    let server = HttpsFixture::start(&published);
    let descriptor: Value = serde_json::from_str(DESCRIPTOR).unwrap();
    let program: ProgramDescriptor = serde_json::from_str(DESCRIPTOR).unwrap();
    let component = &descriptor["components"][0];
    let inventory = component["origin"]["inventory_blake3"].as_str().unwrap();
    let layer = component["layer_id"].as_str().unwrap();
    let game = QualifiedGameId {
        source_id: "creator-live-cycle".into(),
        game_id: "portable".into(),
    };
    publish(&published, &server, &descriptor, "1.5-creator.1");
    let index = server.url("index.json");
    service.source_add(&index).await.unwrap();
    assert!(storage
        .pinned_source_identity(&game.source_id, &index)
        .unwrap()
        .is_some());
    println!(
        "source connected with pinned identity; sandbox {}",
        root.display()
    );
    service
        .game_set_options(&game, options("windowed"))
        .await
        .unwrap();
    let request = InstallRequest {
        external_workspace_root: Some(workspace.to_str().unwrap().to_string()),
        ..InstallRequest::default()
    };
    assert!(!library
        .join(".fusion-source/cas/objects")
        .read_dir()
        .is_ok_and(|mut entries| entries.next().is_some()));
    assert!(storage.get_active_snapshot(&game).unwrap().is_none());
    let install = service.game_install(&game, request.clone()).await;
    let downloaded = evidence
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| event.transfer.as_ref())
        .map(|transfer| transfer.downloaded_bytes)
        .max();
    println!(
        "maximum observed streamed transfer bytes: {downloaded:?}; install result: {install:?}"
    );
    assert_eq!(install.unwrap().status, "completed");
    assert_eq!(downloaded, Some(ZIP_BYTES));
    let first = active(&storage, &game);
    assert!(first.workspace_root.starts_with(&workspace));
    assert!(
        matches!(&first.inputs[layer], LayerInput::Cas { digest, runtime_lock:None } if digest == inventory)
    );
    let cas = CasStore::open(&library).unwrap().verify(inventory).unwrap();
    println!(
        "verified normalized CAS {} ({} files)",
        cas.digest,
        cas.files.len()
    );
    assert_eq!(cas.files.len(), 5763);
    config(&first, true);

    let state = service.game_get_state(&game).await.unwrap();
    assert_eq!(
        service.game_launch(&game).await.unwrap_err().code,
        ErrorCode::UntrustedExecutable
    );
    let approvals: Vec<_> = state
        .executables
        .iter()
        .map(|f| (f.path.clone(), f.blake3.clone()))
        .collect();
    for batch in approvals.chunks(64) {
        service
            .game_trust_executables(&game, batch.to_vec())
            .await
            .unwrap();
    }
    let task = tokio::spawn({
        let service = service.clone();
        let game = game.clone();
        async move { service.game_launch(&game).await }
    });
    let start = Instant::now();
    let executable = first.workspace_root.join(&program.entrypoint.executable);
    let pid = loop {
        if let Some(pid) = process(&executable) {
            break pid;
        }
        if task.is_finished() {
            panic!(
                "launch exited before process observation: {:?}",
                task.await.unwrap()
            );
        }
        assert!(
            start.elapsed() < Duration::from_secs(75),
            "process did not become observable"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    let observed = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(process(&executable), Some(pid));
        if observed.elapsed() >= Duration::from_secs(10) {
            break;
        }
    }
    println!(
        "observed exact candidate PE PID {pid} continuously for {:?}",
        observed.elapsed()
    );
    service.game_cancel(&game).unwrap();
    let stopped = tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(process(&executable), None);

    // Use an inert, non-gameplay private file inside a declared persistence root
    // to verify carry-over without fabricating play or changing upstream bytes.
    let state_probe = Path::new("userdata/addons/fusion-observation.txt");
    fs::write(
        first.workspace_root.join(state_probe),
        b"latest private state",
    )
    .unwrap();
    publish(&published, &server, &descriptor, "1.5-creator.2");
    service.source_refresh(&game.source_id).await.unwrap();
    service
        .game_set_options(&game, options("fullscreen"))
        .await
        .unwrap();
    assert_eq!(active(&storage, &game).id, first.id);
    assert_eq!(
        service.game_update(&game, request).await.unwrap().status,
        "completed"
    );
    let second = active(&storage, &game);
    assert_ne!(second.id, first.id);
    assert_eq!(second.manifest.version, "1.5-creator.2");
    config(&second, false);
    assert_eq!(
        fs::read(second.workspace_root.join(state_probe)).unwrap(),
        b"latest private state"
    );
    fs::write(
        second.workspace_root.join(state_probe),
        b"changed after update",
    )
    .unwrap();
    assert_eq!(
        service.game_rollback(&game).await.unwrap().status,
        "completed"
    );
    let rolled = active(&storage, &game);
    assert_eq!(rolled.id, first.id);
    assert_eq!(rolled.manifest.version, "1.5-creator.1");
    config(&rolled, true);
    assert_eq!(
        fs::read(rolled.workspace_root.join(state_probe)).unwrap(),
        b"changed after update"
    );
    println!("rollback restored first generation and profile, not stale private state");
}
