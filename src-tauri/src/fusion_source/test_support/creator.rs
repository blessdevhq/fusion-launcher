//! H-owned common fixture; no transport/installer implementation lives here.
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use std::time::Duration;

use serde_json::json;

use crate::fusion_source::canonical::QualifiedGameId;
use crate::fusion_source::cas::hash_file;
use crate::fusion_source::dto::InstallRequest;
use crate::fusion_source::fs_safety;
use crate::fusion_source::packer::{PackerEngine, PackerRecipe};
use crate::fusion_source::schema::{DigestAlgorithm, DigestSpec, GameManifest, SourceIndex};
use crate::fusion_source::service::{FusionService, FusionStorage, NoProgress, ServiceEnvironment};
use crate::fusion_source::signatures;
use crate::fusion_source::workspace::WorkspaceSnapshot;
use crate::local_sources::{DesktopSourceTransport, LocalSourceCandidate};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};

pub(crate) const SOURCE_ID: &str = "creator-harness";
const GAME_ID: &str = "authored-demo";
pub(crate) const RUNTIME: &[u8] = b"Fusion-authored synthetic runtime; not executable content\n";
pub(crate) const BASE: &[u8] = b"Fusion-authored synthetic local base\n";
pub(crate) const USER_DATA: &[u8] = b"Fusion-authored separately supplied player data\n";
pub(crate) const CONTENT: &[u8] = b"Fusion-authored tiny managed content archive\n";
#[cfg(windows)]
const NATIVE_HELPER: &str = "fusion_source::supervisor::tests::native::native_process_fixture";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactHashes {
    pub(crate) transport_digest: DigestSpec,
    pub(crate) cas_blake3: String,
    pub(crate) size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublishedBuildHashes {
    /// All relative publication paths, including the signed index, manifests,
    /// raw/ZIP bytes and Packer metadata. Absolute output paths are not evidence.
    pub(crate) files: BTreeMap<String, String>,
    pub(crate) artifacts: BTreeMap<String, ArtifactHashes>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildReproducibility {
    pub(crate) first: PublishedBuildHashes,
    pub(crate) second: PublishedBuildHashes,
}

/// Own this value for the complete scenario; do not retain cloned DB/service
/// handles beyond it. Each construction uses a distinct TempDir and SQLite/CAS.
/// Use `service` directly with the complete `request`; no alternate installer.
pub(crate) struct CreatorHarness {
    pub(crate) service: FusionService,
    pub(crate) storage: Arc<SqliteFusionStorage>,
    pub(crate) game: QualifiedGameId,
    pub(crate) request: InstallRequest,
    pub(crate) author: PathBuf,
    pub(crate) published: PathBuf,
    pub(crate) repeated_publication: PathBuf,
    pub(crate) published_update: PathBuf,
    pub(crate) reproducibility: BuildReproducibility,
    pub(crate) library: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) external_workspace: PathBuf,
    // Last: service/storage release their handles before temporary cleanup.
    _temporary: tempfile::TempDir,
}

impl CreatorHarness {
    /// Small, wholly authored inputs. The runtime is data, not an executable;
    /// this mode tests desktop acquisition without claiming a process launch.
    pub(crate) async fn synthetic() -> Self {
        Self::new(false).await
    }

    /// Reuse the existing supervisor's native child fixture, not game payload.
    #[cfg(windows)]
    pub(crate) async fn native() -> Self {
        Self::new(true).await
    }

    async fn new(native: bool) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let author = root.join("author-private");
        let published = root.join("published-v1");
        let repeated_publication = root.join("published-v1-repeat");
        let published_update = root.join("published-v2");
        let library = root.join("library");
        let database = root.join("player.sqlite");
        let external_workspace = root.join("player-workspaces");
        let user_base = root.join("player-base");
        let user_file = root.join("player-input.bin");
        fs::create_dir_all(author.join("reference-base")).unwrap();
        fs::create_dir_all(author.join("content/data")).unwrap();
        fs::create_dir_all(&user_base).unwrap();
        fs::write(author.join("reference-base/base.bin"), BASE).unwrap();
        fs::write(author.join("content/data/readme.txt"), CONTENT).unwrap();
        fs::write(
            author.join("content/data/settings.ini"),
            b"[authored]\nfixture=true\n",
        )
        .unwrap();
        fs::write(user_base.join("base.bin"), BASE).unwrap();
        fs::write(&user_file, USER_DATA).unwrap();
        let mut layers = vec![
            json!({"id":"base","kind":"base","local_locator":{
                "reference_dir":"reference-base","store":"manual","path_hints":[user_base],
                "key_files":[{"path":"base.bin","role":"core_library"}]}}),
            json!({"id":"runtime","kind":"runtime","local_path":"runtime.bin",
                "archive":"raw","raw_path":"main.exe","mount_path":"runtime","depends_on":["base"],
                "critical_files":[{"path":"main.exe","role":"main_executable"}]}),
            json!({"id":"content-pack","kind":"mod","local_path":"content",
                "archive":"zip","mount_path":"content","depends_on":["base"]}),
        ];
        if native {
            let executable = std::env::current_exe().unwrap();
            fs::copy(&executable, author.join("runtime.bin")).unwrap();
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
        // Build the exact same recipe/inputs/key into two independent empty
        // destinations before modifying input bytes for the update scenario.
        let first = publish(&recipe, &author, &published);
        let second = publish(&recipe, &author, &repeated_publication);
        let reproducibility = BuildReproducibility { first, second };
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
            repeated_publication,
            published_update,
            reproducibility,
            library,
            database,
            external_workspace,
            _temporary: temporary,
        };
        fixture.connect(&fixture.published).await;
        fixture
    }

    pub(crate) async fn connect(&self, directory: &Path) {
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
    pub(crate) async fn approve_and_launch(&self) {
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

    pub(crate) fn index_url(&self) -> String {
        format!("https://offline.fusion.invalid/{SOURCE_ID}/source-index.json")
    }

    pub(crate) fn snapshot(&self) -> WorkspaceSnapshot {
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

/// Streaming hashes of real publication bytes, never of paths/BuildReport alone.
/// This walker is for test-owned directories only; links/reparse points fail.
pub(crate) fn publication_hashes(root: &Path) -> BTreeMap<String, String> {
    fn collect(root: &Path, current: &Path, result: &mut BTreeMap<String, String>) {
        let metadata = fs::symlink_metadata(current).unwrap();
        fs_safety::check_metadata(&metadata, current).unwrap();
        if metadata.is_dir() {
            for entry in fs::read_dir(current).unwrap() {
                collect(root, &entry.unwrap().path(), result);
            }
        } else {
            let relative = current
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            result.insert(
                relative,
                hash_file(current, DigestAlgorithm::Blake3).unwrap(),
            );
        }
    }
    let mut result = BTreeMap::new();
    collect(root, root, &mut result);
    result
}

fn publish(recipe: &serde_json::Value, author: &Path, published: &Path) -> PublishedBuildHashes {
    let recipe: PackerRecipe = serde_json::from_value(recipe.clone()).unwrap();
    let engine = PackerEngine::new();
    let report = engine.build(&recipe, author, published).unwrap();
    assert!(!report.artifacts.is_empty());
    assert_eq!(
        engine.verify(published).unwrap().artifacts_verified,
        report.artifacts.len()
    );
    // Baseline Packer cannot author user_provided recipes. Only this test fixture
    // augments v1.0 wire bytes after verifying Packer's original artifact set.
    // Do not claim Packer verified the augmented manifest/Source.
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
    fs::write(published.join(&relative), &bytes).unwrap();
    fs::write(&report.manifests[GAME_ID], &bytes).unwrap();
    let mut index: SourceIndex =
        serde_json::from_slice(&fs::read(&report.source_index).unwrap()).unwrap();
    index.games[0].manifest_blake3 = digest;
    index.games[0].manifest_url = format!("https://offline.fusion.invalid/{SOURCE_ID}/{relative}");
    // Explicitly public TEST seed; no real private key file is ever read.
    signatures::sign_index(
        &mut index,
        &ed25519_dalek::SigningKey::from_bytes(&[73; 32]),
    )
    .unwrap();
    fs::write(&report.source_index, serde_json::to_vec(&index).unwrap()).unwrap();
    let artifacts = report
        .artifacts
        .iter()
        .map(|artifact| {
            assert_eq!(
                hash_file(&artifact.path, artifact.transport_digest.algorithm).unwrap(),
                artifact.transport_digest.value
            );
            assert_eq!(
                fs::metadata(&artifact.path).unwrap().len(),
                artifact.size_bytes
            );
            (
                artifact.layer_id.clone(),
                ArtifactHashes {
                    transport_digest: artifact.transport_digest.clone(),
                    cas_blake3: artifact.cas_digest.clone(),
                    size_bytes: artifact.size_bytes,
                },
            )
        })
        .collect();
    PublishedBuildHashes {
        files: publication_hashes(published),
        artifacts,
    }
}
