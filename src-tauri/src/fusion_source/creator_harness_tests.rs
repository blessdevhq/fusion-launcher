//! Foundation H: real desktop transport + existing Packer/service/SQLite wiring.
//! No transport trait is implemented here. Local acquisition is not HTTPS QA.
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use super::canonical::QualifiedGameId;
use super::cas::hash_file;
use super::dto::InstallRequest;
use super::errors::ErrorCode;
use super::packer::{PackerEngine, PackerRecipe};
use super::schema::{DigestAlgorithm, GameManifest, SourceIndex};
use super::service::{FusionService, FusionStorage, NoProgress, ServiceEnvironment};
use super::workspace::WorkspaceSnapshot;
use crate::local_sources::{DesktopSourceTransport, LocalSourceCandidate};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};

const SOURCE_ID: &str = "creator-harness";
const GAME_ID: &str = "authored-demo";
const RUNTIME: &[u8] = b"Fusion-authored synthetic runtime; not executable content\n";
const BASE: &[u8] = b"Fusion-authored synthetic local base\n";
const USER_DATA: &[u8] = b"Fusion-authored separately supplied player data\n";
#[cfg(windows)]
const NATIVE_HELPER: &str = "fusion_source::supervisor::tests::native::native_process_fixture";

#[tokio::test]
async fn fixture_supplies_the_complete_install_request() {
    let fixture = CreatorHarness::new(false).await;
    assert!(fixture.request.user_files.contains_key("user-data"));
    assert_eq!(fixture.request.raw_paths["runtime"], "main.exe");
    assert_eq!(fixture.request.raw_paths["user-data"], "player.bin");
    assert_eq!(
        fixture.request.external_workspace_root.as_deref(),
        Some(fixture.external_workspace.to_str().unwrap())
    );
    assert!(!fixture.author.exists(), "author inputs are unavailable");
    assert!(fixture
        .storage
        .get_active_snapshot(&fixture.game)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn desktop_transport_clean_install_consumes_published_and_player_inputs() {
    let fixture = CreatorHarness::new(false).await;
    let result = fixture
        .service
        .game_install(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    let snapshot = fixture.snapshot();
    assert!(snapshot
        .workspace_root
        .starts_with(&fixture.external_workspace));
    assert!(!snapshot.workspace_root.starts_with(&fixture.published));
    assert_eq!(
        fs::read(snapshot.workspace_root.join("runtime/main.exe")).unwrap(),
        RUNTIME
    );
    assert_eq!(
        fs::read(snapshot.workspace_root.join("base.bin")).unwrap(),
        BASE
    );
    assert_eq!(
        fs::read(snapshot.workspace_root.join("user/player.bin")).unwrap(),
        USER_DATA
    );
    let state = fixture.service.game_get_state(&fixture.game).await.unwrap();
    assert!(
        !state.ready,
        "a source signature never grants executable approval"
    );
    assert!(!state.executables.is_empty());
    assert!(fixture
        .storage
        .pinned_source_identity(SOURCE_ID, &fixture.index_url())
        .unwrap()
        .is_some());
    assert!(fixture.database.is_file());
    assert!(fixture.library.is_dir());
    assert_eq!(
        fs::read(&fixture.request.user_files["user-data"]).unwrap(),
        USER_DATA
    );
}

#[cfg(windows)]
#[tokio::test]
async fn native_desktop_install_launch_update_rollback_preserves_latest_save() {
    let fixture = CreatorHarness::new(true).await;
    assert!(!fixture.author.exists());
    fixture
        .service
        .game_install(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    let initial = fixture.snapshot();
    assert_eq!(
        fixture
            .service
            .game_launch(&fixture.game)
            .await
            .unwrap_err()
            .code,
        ErrorCode::UntrustedExecutable
    );
    fixture.approve_and_launch().await;
    let save = initial.workspace_root.join("saves/progress.dat");
    assert_eq!(fs::read(&save).unwrap(), b"new atomic save");
    fs::write(&save, b"player progress after initial launch").unwrap();

    fixture.connect(&fixture.published_update).await;
    let update = fixture
        .service
        .game_update(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    assert_eq!(update.status, "completed");
    let updated = fixture.snapshot();
    assert_ne!(updated.id, initial.id);
    assert_eq!(updated.manifest.version, "2");
    assert!(updated.delta.reused.contains(&"base".into()));
    assert_eq!(
        fs::read(updated.workspace_root.join("saves/progress.dat")).unwrap(),
        b"player progress after initial launch"
    );
    assert!(
        !fixture
            .service
            .game_get_state(&fixture.game)
            .await
            .unwrap()
            .ready,
        "changed runtime needs fresh approval"
    );
    assert_ne!(
        hash_file(
            &initial.workspace_root.join("runtime/main.exe"),
            DigestAlgorithm::Blake3
        )
        .unwrap(),
        hash_file(
            &updated.workspace_root.join("runtime/main.exe"),
            DigestAlgorithm::Blake3
        )
        .unwrap()
    );
    fixture.approve_and_launch().await;
    fs::write(
        updated.workspace_root.join("saves/progress.dat"),
        b"latest player progress after update",
    )
    .unwrap();

    let rollback = fixture.service.game_rollback(&fixture.game).await.unwrap();
    assert_eq!(rollback.status, "completed");
    let rolled = fixture.snapshot();
    assert_eq!(rolled.id, initial.id);
    assert_eq!(
        fs::read(rolled.workspace_root.join("saves/progress.dat")).unwrap(),
        b"latest player progress after update"
    );
    assert!(
        fixture
            .service
            .game_get_state(&fixture.game)
            .await
            .unwrap()
            .ready
    );
    fixture.approve_and_launch().await;
    assert_eq!(
        fs::read(rolled.workspace_root.join("saves/progress.dat")).unwrap(),
        b"new atomic save"
    );
    assert!(
        !fixture.author.exists(),
        "no launch can fall back to the author input"
    );
}

/// Shared test-only foundation; later streams can extend this instead of
/// implementing another installer or replacing the desktop transport.
pub(super) struct CreatorHarness {
    pub(super) service: FusionService,
    pub(super) storage: Arc<SqliteFusionStorage>,
    pub(super) game: QualifiedGameId,
    pub(super) request: InstallRequest,
    pub(super) author: PathBuf,
    pub(super) published: PathBuf,
    pub(super) published_update: PathBuf,
    pub(super) library: PathBuf,
    pub(super) database: PathBuf,
    pub(super) external_workspace: PathBuf,
    // Last: service/storage release their handles before temporary cleanup.
    _temporary: tempfile::TempDir,
}

impl CreatorHarness {
    pub(super) async fn new(native: bool) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let author = root.join("author-private");
        let published = root.join("published-v1");
        let published_update = root.join("published-v2");
        let library = root.join("library");
        let database = root.join("player.sqlite");
        let external_workspace = root.join("player-workspaces");
        let user_base = root.join("player-base");
        let user_file = root.join("player-input.bin");
        fs::create_dir_all(author.join("reference-base")).unwrap();
        fs::create_dir_all(&user_base).unwrap();
        fs::write(author.join("reference-base/base.bin"), BASE).unwrap();
        fs::write(user_base.join("base.bin"), BASE).unwrap();
        fs::write(&user_file, USER_DATA).unwrap();
        let mut layers = vec![
            json!({"id":"base","kind":"base","local_locator":{
                "reference_dir":"reference-base","store":"manual","path_hints":[user_base],
                "key_files":[{"path":"base.bin","role":"core_library"}]}}),
            json!({"id":"runtime","kind":"runtime","local_path":"runtime.bin",
                "archive":"raw","raw_path":"main.exe","mount_path":"runtime","depends_on":["base"],
                "critical_files":[{"path":"main.exe","role":"main_executable"}]}),
        ];
        if native {
            let executable = std::env::current_exe().unwrap();
            fs::copy(&executable, author.join("runtime.bin")).unwrap();
            // Reuse the supervisor fixture, including the external Windows
            // activation manifest required by a renamed Rust/Tauri test image.
            let activation = PathBuf::from(format!("{}.manifest", executable.display()));
            if activation.is_file() {
                fs::copy(activation, author.join("activation.manifest")).unwrap();
                layers.push(json!({"id":"activation","kind":"runtime","local_path":"activation.manifest",
                    "archive":"raw","raw_path":"main.exe.manifest","mount_path":"runtime","depends_on":["base"],
                    "critical_files":[{"path":"main.exe.manifest","role":"core_library"}]}));
            }
        } else {
            fs::write(author.join("runtime.bin"), RUNTIME).unwrap();
        }
        let mut recipe = json!({
            "source_info":{"id":SOURCE_ID,"name":"Authored Creator foundation fixture"},
            "base_url":format!("https://offline.fusion.invalid/{SOURCE_ID}/"),
            "games":[{"id":GAME_ID,"title":"Synthetic authored fixture","version":"1",
                "local_placement":"workspace_on_base_volume","layers":layers,
                "runtime":{"layer_id":"runtime","executable":"runtime/main.exe"},
                "persistence":{"saves_paths":["workspace/saves"]}
            }]
        });
        #[cfg(windows)]
        if native {
            recipe["games"][0]["runtime"]["args"] =
                json!(["--exact", NATIVE_HELPER, "--nocapture"]);
            recipe["games"][0]["runtime"]["env"] = json!({
                "FUSION_SUPERVISOR_FIXTURE":"delayed",
                "FUSION_SAVE":"{{WORKSPACE_DIR}}/saves/progress.dat"
            });
        }
        publish(&recipe, &author, &published);
        // A trailing PE overlay changes the exact executable hash while keeping
        // the same owned native test program executable; no game binary is used.
        fs::OpenOptions::new()
            .append(true)
            .open(author.join("runtime.bin"))
            .unwrap()
            .write_all(b"\nFusion authored synthetic release two\n")
            .unwrap();
        recipe["games"][0]["version"] = json!("2");
        publish(&recipe, &author, &published_update);
        fs::remove_dir_all(&author).unwrap();

        let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(
            RepositoryStore::open(&database).unwrap(),
        ))));
        let service = FusionService::open(
            &library,
            storage.clone(),
            Arc::new(DesktopSourceTransport::new(storage.clone())),
            Arc::new(NoProgress),
            environment(root),
        )
        .unwrap();
        let mut raw_paths = BTreeMap::from([
            ("runtime".into(), "main.exe".into()),
            ("user-data".into(), "player.bin".into()),
        ]);
        if recipe["games"][0]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|layer| layer["id"] == "activation")
        {
            raw_paths.insert("activation".into(), "main.exe.manifest".into());
        }
        let request = InstallRequest {
            user_files: BTreeMap::from([("user-data".into(), user_file.to_str().unwrap().into())]),
            raw_paths,
            external_workspace_root: Some(external_workspace.to_str().unwrap().into()),
        };
        let fixture = Self {
            service,
            storage,
            game: QualifiedGameId {
                source_id: SOURCE_ID.into(),
                game_id: GAME_ID.into(),
            },
            request,
            author,
            published,
            published_update,
            library,
            database,
            external_workspace,
            _temporary: temporary,
        };
        fixture.connect(&fixture.published).await;
        fixture
    }

    pub(super) async fn connect(&self, directory: &Path) {
        let candidate = LocalSourceCandidate::open(
            directory,
            &self.library,
            self.storage.clone(),
            environment(self._temporary.path()),
        )
        .unwrap();
        assert!(candidate.preview().await.unwrap().source.signature_verified);
        candidate
            .connect(&self.storage, SOURCE_ID, false)
            .await
            .unwrap();
    }

    #[cfg(windows)]
    pub(super) async fn approve_and_launch(&self) {
        let state = self.service.game_get_state(&self.game).await.unwrap();
        self.service
            .game_trust_executables(
                &self.game,
                state
                    .executables
                    .iter()
                    .map(|file| (file.path.clone(), file.blake3.clone()))
                    .collect(),
            )
            .await
            .unwrap();
        let launch = tokio::time::timeout(
            Duration::from_secs(60),
            self.service.game_launch(&self.game),
        )
        .await
        .expect("bounded native fixture launch")
        .unwrap();
        assert_eq!(launch.status, "completed");
        let after = self.service.game_get_state(&self.game).await.unwrap();
        if let Some(report) = after.crash_reports.first() {
            panic!(
                "native fixture failed: {:?}",
                self.service.crash_preview(&report.report_id).unwrap()
            );
        }
        assert!(
            after
                .operation
                .as_ref()
                .is_none_or(|operation| operation.status != "running"),
            "game_launch waited for process exit/save reconciliation"
        );
    }

    pub(super) fn index_url(&self) -> String {
        format!("https://offline.fusion.invalid/{SOURCE_ID}/source-index.json")
    }

    pub(super) fn snapshot(&self) -> WorkspaceSnapshot {
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

fn environment(root: &Path) -> ServiceEnvironment {
    ServiceEnvironment {
        appdata: root.join("appdata"),
        local_appdata: root.join("local-appdata"),
        display_width: 1920,
        display_height: 1080,
        refresh_rate: 60,
    }
}

fn publish(recipe: &serde_json::Value, author: &Path, published: &Path) {
    let recipe: PackerRecipe = serde_json::from_value(recipe.clone()).unwrap();
    let engine = PackerEngine::new();
    let report = engine.build(&recipe, author, published).unwrap();
    assert!(!report.artifacts.is_empty());
    assert_eq!(
        engine.verify(published).unwrap().artifacts_verified,
        report.artifacts.len()
    );
    // Baseline Packer cannot author user_provided recipes yet. Extend only this
    // TEST fixture using the existing 1.0 wire contract. This is not a Creator
    // recipe compiler or a claim that Packer verified the augmented publication.
    let mut manifest: GameManifest =
        serde_json::from_slice(&fs::read(&report.manifests[GAME_ID]).unwrap()).unwrap();
    manifest.layers.push(
        serde_json::from_value(json!({
            "id":"user-data","kind":"mod","mount_path":"user",
            "source":{"kind":"user_provided","archive":"raw"},"depends_on":["base"]
        }))
        .unwrap(),
    );
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let digest = blake3::hash(&bytes).to_hex().to_string();
    let relative = format!("manifests/{GAME_ID}-{digest}.json");
    // BuildReport.manifests points to a convenience alias. The service fetches
    // the immutable index URL, so publish new digest-named bytes and rebind it.
    fs::write(published.join(&relative), &bytes).unwrap();
    fs::write(&report.manifests[GAME_ID], &bytes).unwrap();
    let mut index: SourceIndex =
        serde_json::from_slice(&fs::read(&report.source_index).unwrap()).unwrap();
    index.games[0].manifest_blake3 = digest;
    index.games[0].manifest_url = format!("https://offline.fusion.invalid/{SOURCE_ID}/{relative}");
    // Explicitly public TEST signing material; no real private key is read.
    super::signatures::sign_index(
        &mut index,
        &ed25519_dalek::SigningKey::from_bytes(&[73; 32]),
    )
    .unwrap();
    fs::write(&report.source_index, serde_json::to_vec(&index).unwrap()).unwrap();
}
