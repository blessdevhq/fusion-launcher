//! Source-authoring round trips through the same contracts used by the launcher.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::adapters::{ArtifactTransport, Cancellation, TransferLimits, TransferReceipt};
use super::canonical::{CanonicalGameManifest, QualifiedGameId};
use super::capabilities::CapabilityRegistry;
use super::config_templates::TemplateContext;
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::integrity::{ExecutableTrust, PreLaunchIntegrityGuard};
use super::layers::{LayerCache, LayerInput};
use super::packer::{BuildReport, PackerEngine, PackerRecipe};
use super::persistence::PersistenceRoots;
use super::plan::{PlanBuilder, PlanOptions};
use super::resolver::{ManifestFetcher, VerifiedManifest};
use super::schema::SourceIndex;
use super::workspace::{AssembledWorkspace, WorkspaceAssembler, WorkspaceBuildOptions};

const SOURCE_URL: &str = "https://author.example/";
const INDEX_URL: &str = "https://author.example/source-index.json";
const GAME_ID: &str = "packer-game";
const SOURCE_ID: &str = "packer-source";
const HELPER: &str = "fusion_source::packer_tests::packer_native_process_fixture";

#[tokio::test]
async fn signing_keygen_build_verify_fetch_and_incremental_resigning_round_trip() {
    let mut fixture = Fixture::new(false);
    let key = fixture.temporary.path().join("signing.pem");
    let generated = super::signatures::keygen(&key).unwrap();
    let engine = PackerEngine::new();
    engine
        .build_with_signing_key(
            &fixture.typed_recipe(),
            fixture.temporary.path(),
            &fixture.dist,
            Some(&key),
        )
        .unwrap();
    assert_eq!(
        fixture.index().source_info.public_key.as_deref(),
        Some(generated.public_key.as_str())
    );
    assert!(fixture.index().signature.is_some());
    assert!(engine.verify(&fixture.dist).unwrap().signature_verified);
    fixture.fetch().await;

    // Incremental publication retains the complete previous index and signs it
    // again, rather than signing only the new recipe's game entries.
    let first_signature = fixture.index().signature.unwrap();
    fixture.recipe["games"][0]["id"] = json!("second-game");
    engine
        .build_with_signing_key(
            &fixture.typed_recipe(),
            fixture.temporary.path(),
            &fixture.dist,
            Some(&key),
        )
        .unwrap();
    assert_eq!(fixture.index().games.len(), 2);
    assert_ne!(
        fixture.index().signature.as_deref(),
        Some(first_signature.as_str())
    );
    assert_eq!(engine.verify(&fixture.dist).unwrap().manifests_verified, 2);

    let published = fs::read(fixture.dist.join("source-index.json")).unwrap();
    let other_key = fixture.temporary.path().join("other.pem");
    super::signatures::keygen(&other_key).unwrap();
    assert_eq!(
        engine
            .build_with_signing_key(
                &fixture.typed_recipe(),
                fixture.temporary.path(),
                &fixture.dist,
                Some(&other_key)
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidSourceSignature
    );
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );

    let mut changed: Value = serde_json::from_slice(&published).unwrap();
    changed["games"][0]["manifest_url"] = json!("missing-manifest.json");
    fs::write(
        fixture.dist.join("source-index.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert_eq!(
        engine.verify(&fixture.dist).unwrap_err().code,
        ErrorCode::InvalidSourceSignature
    );
    assert_eq!(
        engine
            .build_with_signing_key(
                &fixture.typed_recipe(),
                fixture.temporary.path(),
                &fixture.dist,
                Some(&key)
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidSourceSignature
    );
}

#[test]
fn unsigned_output_omits_signature_and_keyed_recipe_requires_matching_key() {
    let mut fixture = Fixture::new(false);
    fixture.build().unwrap();
    let published = fs::read(fixture.dist.join("source-index.json")).unwrap();
    let wire: Value = serde_json::from_slice(&published).unwrap();
    assert!(wire.get("signature").is_none());
    assert!(
        !PackerEngine::new()
            .verify(&fixture.dist)
            .unwrap()
            .signature_verified
    );
    let key = fixture.temporary.path().join("private.pem");
    let generated = super::signatures::keygen(&key).unwrap();
    fixture.recipe["source_info"]["public_key"] = json!(generated.public_key);
    assert_eq!(
        fixture.build().unwrap_err().code,
        ErrorCode::InvalidSourceSignature
    );
    fixture.recipe["source_info"]["public_key"] =
        json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    assert_eq!(
        PackerEngine::new()
            .build_with_signing_key(
                &fixture.typed_recipe(),
                fixture.temporary.path(),
                &fixture.dist,
                Some(&key)
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidSourceSignature
    );
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
}

/// Use a real test executable without introducing a compiler or shell dependency.
/// The child is selected by its exact test name, so it cannot recursively run the suite.
#[test]
fn packer_native_process_fixture() {
    if std::env::var("FUSION_PACKER_TEST_PROCESS").as_deref() != Ok("1") {
        return;
    }
    println!("packer end-to-end process marker");
    eprintln!("packer end-to-end stderr marker");
}

struct Fixture {
    temporary: tempfile::TempDir,
    dist: PathBuf,
    library: PathBuf,
    recipe: Value,
}

impl Fixture {
    fn new(native: bool) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let base = root.join("base");
        let runtime = root.join("runtime");
        let addon = root.join("addon");
        for path in [
            &base.join("data/config"),
            &base.join("saves"),
            &runtime,
            &addon,
        ] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(base.join("data/content.bin"), b"owned test game data").unwrap();
        fs::write(base.join("data/core.dll"), b"test core library").unwrap();
        fs::write(base.join("data/config/state.ini"), b"private state").unwrap();
        fs::write(base.join("saves/progress.dat"), b"original progress").unwrap();
        fs::write(addon.join("injector.dll"), b"owned test injector bytes").unwrap();
        let mut critical = vec![json!({"path":"main.exe","role":"main_executable"})];
        if native {
            let executable = std::env::current_exe().unwrap();
            fs::copy(&executable, runtime.join("main.exe")).unwrap();
            // The renamed Windows test image may need its external activation manifest.
            let activation = PathBuf::from(format!("{}.manifest", executable.display()));
            if activation.is_file() {
                fs::copy(activation, runtime.join("main.exe.manifest")).unwrap();
                critical.push(json!({"path":"main.exe.manifest","role":"core_library"}));
            }
        } else {
            fs::write(runtime.join("main.exe"), b"test runtime version one").unwrap();
        }
        zip_directory(&runtime, &root.join("runtime.zip"));
        let recipe = json!({
            "source_info":{"id":SOURCE_ID,"name":"Owned authoring fixture","maintainer":"Fusion tests"},
            "base_url":SOURCE_URL,
            "games":[{
                "id":GAME_ID,"title":"Authoring round trip","version":"1.0.0","platform":"pc",
                "layers":[
                    {"id":"base","kind":"base","local_path":"base","archive":"zip",
                     "mirrors":["https://author.example/artifacts/{{ARTIFACT_FILENAME}}"],
                     "critical_files":[{"path":"data/core.dll","role":"core_library"}],
                     "writable_copy_paths":["data/config/state.ini"]},
                    {"id":"runtime","kind":"runtime","local_path":"runtime.zip","archive":"zip",
                     "mount_path":"runtime","depends_on":["base"],
                     "mirrors":["https://author.example/artifacts/{{ARTIFACT_FILENAME}}"],
                     "critical_files":critical},
                    {"id":"injector","kind":"mod","local_path":"addon","archive":"zip",
                     "mount_path":"mods","depends_on":["base"],"optional":true,"default_enabled":true,
                     "mirrors":["https://author.example/artifacts/{{ARTIFACT_FILENAME}}"],
                     "critical_files":[{"path":"injector.dll","role":"injector_dll"}]}
                ],
                "runtime":{"layer_id":"runtime","executable":"runtime/main.exe",
                    "args":["--exact",HELPER,"--nocapture"],"env":{"FUSION_PACKER_TEST_PROCESS":"1"}},
                "persistence":{"saves_paths":["workspace/saves/progress.dat"]},
                "config_templates":[
                    {"id":"display","path":"data/config/generated.toml","format":"toml",
                     "template":"[display]\nwidth = {{DISPLAY_WIDTH}}\nheight = {{DISPLAY_HEIGHT}}\n"},
                    {"id":"refresh","path":"data/config/generated.json","format":"json",
                     "template":"{\"refresh_rate\": {{REFRESH_RATE}}}"}
                ]
            }]
        });
        Self {
            dist: root.join("dist"),
            library: root.join("library"),
            temporary,
            recipe,
        }
    }

    fn typed_recipe(&self) -> PackerRecipe {
        serde_json::from_value(self.recipe.clone()).unwrap()
    }

    fn build(&self) -> Result<BuildReport, FusionError> {
        PackerEngine::new().build(&self.typed_recipe(), self.temporary.path(), &self.dist)
    }

    fn index(&self) -> SourceIndex {
        serde_json::from_slice(&fs::read(self.dist.join("source-index.json")).unwrap()).unwrap()
    }

    async fn fetch(&self) -> VerifiedManifest {
        let fetcher = ManifestFetcher::open(
            &self.library,
            Arc::new(LocalRepositoryTransport(self.dist.clone())),
        )
        .unwrap();
        let index = fetcher
            .fetch_index(INDEX_URL, &Cancellation::default())
            .await
            .unwrap();
        let mut entry = index
            .games
            .iter()
            .find(|entry| entry.id == GAME_ID)
            .unwrap()
            .clone();
        // Source loading resolves relative links against the index URL in production.
        entry.manifest_url = url::Url::parse(INDEX_URL)
            .unwrap()
            .join(&entry.manifest_url)
            .unwrap()
            .to_string();
        fetcher
            .fetch_manifest(
                SOURCE_ID,
                &entry,
                &BTreeMap::new(),
                &Cancellation::default(),
            )
            .await
            .unwrap()
    }

    fn inputs(
        &self,
        report: &BuildReport,
        manifest: &CanonicalGameManifest,
    ) -> BTreeMap<String, LayerInput> {
        let cache = LayerCache::open(&self.library).unwrap();
        report
            .artifacts
            .iter()
            .filter(|artifact| artifact.game_id == GAME_ID)
            .map(|artifact| {
                let layer = &manifest.layers[&artifact.layer_id];
                let input = cache
                    .prepare_artifact(layer, &artifact.path, artifact.raw_path.as_deref(), None)
                    .unwrap();
                let object = cache.object(&input).unwrap();
                assert_eq!(object.digest, artifact.cas_digest);
                cache.cas().verify(&object.digest).unwrap();
                (artifact.layer_id.clone(), input)
            })
            .collect()
    }

    fn assemble(
        &self,
        assembler: &WorkspaceAssembler,
        verified: &VerifiedManifest,
        inputs: &BTreeMap<String, LayerInput>,
    ) -> AssembledWorkspace {
        let appdata = self.temporary.path().join("appdata");
        let local = self.temporary.path().join("localappdata");
        for path in [&appdata, &local] {
            fs::create_dir_all(path).unwrap();
        }
        let plan = PlanBuilder::build(
            &verified.canonical,
            &PlanOptions::default(),
            &CapabilityRegistry::current_client().with("proc.supervisor.v1"),
        )
        .unwrap();
        assembler
            .assemble(
                &verified.canonical,
                &plan,
                inputs,
                &TemplateContext {
                    workspace_dir: self.library.clone(),
                    appdata_dir: appdata.clone(),
                    local_appdata_dir: local.clone(),
                    display_width: 1920,
                    display_height: 1080,
                    refresh_rate: 144,
                    enabled_addons: BTreeSet::from(["injector".into()]),
                },
                &PersistenceRoots {
                    appdata,
                    local_appdata: local,
                },
                &WorkspaceBuildOptions {
                    verified_manifest: Some(verified.snapshot_binding()),
                    ..Default::default()
                },
            )
            .unwrap()
    }
}

/// Local test transport still exercises ManifestFetcher byte limits and raw hash binding.
struct LocalRepositoryTransport(PathBuf);

#[async_trait]
impl ArtifactTransport for LocalRepositoryTransport {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        cancel.check()?;
        let url = url::Url::parse(url).unwrap();
        assert_eq!(url.host_str(), Some("author.example"));
        let relative = fs_safety::validate_relative_path(url.path().trim_start_matches('/'))?;
        let source = self.0.join(relative);
        let mut input = File::open(&source).unwrap();
        let size = input.metadata().unwrap().len();
        if size > limits.max_bytes {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "Fixture metadata exceeds its transfer budget.",
            ));
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .unwrap();
        let bytes = std::io::copy(&mut input, &mut output).unwrap();
        drop(output);
        cancel.check()?;
        Ok(TransferReceipt {
            bytes,
            final_url: url.to_string(),
        })
    }
}

#[derive(Default)]
struct ExactTrust {
    expected: BTreeMap<String, String>,
    checked: Mutex<BTreeSet<String>>,
}

impl ExactTrust {
    fn for_manifest(manifest: &CanonicalGameManifest) -> Self {
        let expected = manifest
            .layers
            .values()
            .flat_map(|layer| {
                layer.critical_files.iter().map(move |file| {
                    (
                        super::evolution::mounted_path(&layer.mount_path, &file.path),
                        file.blake3.clone(),
                    )
                })
            })
            .collect();
        Self {
            expected,
            ..Default::default()
        }
    }
}

impl ExecutableTrust for ExactTrust {
    fn is_trusted(
        &self,
        game: &QualifiedGameId,
        path: &str,
        blake3: &str,
    ) -> Result<bool, FusionError> {
        self.checked.lock().unwrap().insert(path.into());
        Ok(game.source_id == SOURCE_ID
            && game.game_id == GAME_ID
            && self
                .expected
                .get(path)
                .is_some_and(|expected| expected == blake3))
    }
}

fn zip_directory(directory: &Path, destination: &Path) {
    let mut writer = zip::ZipWriter::new(File::create(destination).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for file in entries {
        assert!(file.is_file());
        writer
            .start_file(file.file_name().unwrap().to_str().unwrap(), options)
            .unwrap();
        std::io::copy(&mut File::open(&file).unwrap(), &mut writer).unwrap();
    }
    writer.finish().unwrap();
}

#[tokio::test]
async fn packer_round_trip_fetch_cas_workspace_integrity_supervision_update_and_rollback() {
    let mut fixture = Fixture::new(true);
    fs::write(
        fixture.temporary.path().join("unrelated.exe"),
        b"unrelated test executable",
    )
    .unwrap();
    fixture.recipe["games"].as_array_mut().unwrap().push(json!({
        "id":"unrelated-game","title":"Unrelated entry","version":"4.2",
        "layers":[{"id":"base","kind":"base","local_path":"unrelated.exe","raw_path":"main.exe",
            "base_url":"https://author.example/artifacts/","critical_files":[{"path":"main.exe","role":"main_executable"}]}],
        "launch":{"id":"other","executable":"main.exe","role":"primary"}
    }));
    let first_build = fixture.build().unwrap();
    PackerEngine::new().verify(&fixture.dist).unwrap();
    let first_index = fixture.index();
    let untouched = first_index
        .games
        .iter()
        .find(|entry| entry.id == "unrelated-game")
        .unwrap()
        .clone();
    let first = fixture.fetch().await;
    assert_eq!(
        first.raw_blake3,
        blake3::hash(&first.raw_bytes).to_hex().as_str()
    );
    assert_eq!(
        first.raw_bytes,
        fs::read(fixture.dist.join("manifests/packer-game.json")).unwrap()
    );
    let first_inputs = fixture.inputs(&first_build, &first.canonical);
    let cache = LayerCache::open(&fixture.library).unwrap();
    let base = cache.object(&first_inputs["base"]).unwrap();
    let base_identity = fs_safety::file_identity(&base.root.join("data/content.bin")).unwrap();
    let assembler = WorkspaceAssembler::open(
        &fixture.library,
        &QualifiedGameId {
            source_id: SOURCE_ID.into(),
            game_id: GAME_ID.into(),
        },
    )
    .unwrap();
    let first_workspace = fixture.assemble(&assembler, &first, &first_inputs);
    let first_id = first_workspace.snapshot.id.clone();
    assert_eq!(
        first_workspace.snapshot.source_manifest_blake3.as_deref(),
        Some(first.raw_blake3.as_str())
    );
    let root = &first_workspace.snapshot.workspace_root;
    let rendered: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("data/config/generated.toml")).unwrap())
            .unwrap();
    assert_eq!(rendered["display"]["width"].as_integer(), Some(1920));
    let rendered: Value =
        serde_json::from_slice(&fs::read(root.join("data/config/generated.json")).unwrap())
            .unwrap();
    assert_eq!(rendered["refresh_rate"], 144);
    assert_eq!(
        fs_safety::file_identity(&root.join("data/content.bin")).unwrap(),
        base_identity
    );
    let trust = Arc::new(ExactTrust::for_manifest(&first.canonical));
    PreLaunchIntegrityGuard::verify(&first_workspace.snapshot, trust.as_ref()).unwrap();
    assert_eq!(
        *trust.checked.lock().unwrap(),
        trust.expected.keys().cloned().collect::<BTreeSet<_>>()
    );
    assert_eq!(
        PreLaunchIntegrityGuard::verify(&first_workspace.snapshot, &ExactTrust::default())
            .unwrap_err()
            .code,
        ErrorCode::UntrustedExecutable
    );
    #[cfg(windows)]
    {
        use super::supervisor::{ProcessSupervisor, SupervisorOptions};
        let running =
            ProcessSupervisor::launch(first_workspace, trust, None, SupervisorOptions::default())
                .await
                .unwrap();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), running.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.primary_exit_code, 0);
        assert!(!outcome.user_cancelled);
        assert!(outcome
            .log_tail
            .iter()
            .any(|line| line.text.contains("packer end-to-end process marker")));
    }
    #[cfg(not(windows))]
    drop(first_workspace);
    let active = assembler.active().unwrap().unwrap();
    fs_safety::atomic_write(
        &active.snapshot.workspace_root.join("saves/progress.dat"),
        b"progress before update",
    )
    .unwrap();
    drop(active);

    // A second binary build changes the archive and critical hash, retaining a valid PE image.
    OpenOptions::new()
        .append(true)
        .open(fixture.temporary.path().join("runtime/main.exe"))
        .unwrap()
        .write_all(b"stage-5a-runtime-build-two")
        .unwrap();
    zip_directory(
        &fixture.temporary.path().join("runtime"),
        &fixture.temporary.path().join("runtime.zip"),
    );
    fixture.recipe["games"].as_array_mut().unwrap().truncate(1);
    fixture.recipe["games"][0]["version"] = json!("1.0.1");
    let second_build = fixture.build().unwrap();
    PackerEngine::new().verify(&fixture.dist).unwrap();
    let index = fixture.index();
    assert_eq!(
        index
            .games
            .iter()
            .find(|entry| entry.id == "unrelated-game"),
        Some(&untouched)
    );
    let second = fixture.fetch().await;
    assert_ne!(first.raw_blake3, second.raw_blake3);
    assert_ne!(
        first.canonical.layers["runtime"].critical_files[0].blake3,
        second.canonical.layers["runtime"].critical_files[0].blake3
    );
    let second_inputs = fixture.inputs(&second_build, &second.canonical);
    assert_eq!(first_inputs["base"], second_inputs["base"]);
    assert_eq!(
        fs_safety::file_identity(
            &cache
                .object(&second_inputs["base"])
                .unwrap()
                .root
                .join("data/content.bin")
        )
        .unwrap(),
        base_identity
    );
    let second_workspace = fixture.assemble(&assembler, &second, &second_inputs);
    assert_ne!(second_workspace.snapshot.id, first_id);
    assert_eq!(second_workspace.snapshot.delta.changed, ["runtime"]);
    assert_eq!(second_workspace.snapshot.delta.reused, ["base", "injector"]);
    PreLaunchIntegrityGuard::verify(
        &second_workspace.snapshot,
        &ExactTrust::for_manifest(&second.canonical),
    )
    .unwrap();
    fs_safety::atomic_write(
        &second_workspace
            .snapshot
            .workspace_root
            .join("saves/progress.dat"),
        b"progress after update",
    )
    .unwrap();
    drop(second_workspace);
    let rolled_back = assembler.rollback().unwrap();
    assert_eq!(rolled_back.snapshot.id, first_id);
    assert_eq!(rolled_back.snapshot.inputs, first_inputs);
    assert_eq!(
        rolled_back.snapshot.source_manifest_blake3.as_deref(),
        Some(first.raw_blake3.as_str())
    );
    assert_eq!(
        fs::read(
            rolled_back
                .snapshot
                .workspace_root
                .join("saves/progress.dat")
        )
        .unwrap(),
        b"progress after update"
    );
    PreLaunchIntegrityGuard::verify(
        &rolled_back.snapshot,
        &ExactTrust::for_manifest(&first.canonical),
    )
    .unwrap();
}

#[test]
fn packer_is_deterministic_and_verify_detects_changed_manifest_or_artifact_bytes() {
    let mut fixture = Fixture::new(false);
    fixture.recipe["games"][0]["layers"][0]["local_path"] = json!("./base");
    let json_recipe = fixture.temporary.path().join("./recipe.json");
    let toml_recipe = fixture.temporary.path().join("recipe.toml");
    fs::write(
        &json_recipe,
        serde_json::to_vec_pretty(&fixture.recipe).unwrap(),
    )
    .unwrap();
    fs::write(
        &toml_recipe,
        toml::to_string(&fixture.typed_recipe()).unwrap(),
    )
    .unwrap();
    let first = PackerEngine::new()
        .build_file(&json_recipe, &fixture.dist)
        .unwrap();
    let other_dist = fixture.temporary.path().join("second-dist");
    let second = PackerEngine::new()
        .build_file(&toml_recipe, &other_dist)
        .unwrap();
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        fs::read(other_dist.join("source-index.json")).unwrap()
    );
    for artifact in &first.artifacts {
        let other = second
            .artifacts
            .iter()
            .find(|other| other.layer_id == artifact.layer_id)
            .unwrap();
        assert_eq!(artifact.transport_digest, other.transport_digest);
        assert_eq!(artifact.cas_digest, other.cas_digest);
        assert_eq!(
            fs::read(&artifact.path).unwrap(),
            fs::read(&other.path).unwrap()
        );
    }
    let index = fixture.index();
    let url = url::Url::parse(&index.games[0].manifest_url).unwrap();
    let manifest = fixture.dist.join(url.path().trim_start_matches('/'));
    let original = fs::read(&manifest).unwrap();
    let mut reformatted = original.clone();
    reformatted.push(b'\n');
    fs::write(&manifest, reformatted).unwrap();
    assert_eq!(
        PackerEngine::new().verify(&fixture.dist).unwrap_err().code,
        ErrorCode::ManifestDigestMismatch
    );
    fs::write(&manifest, original).unwrap();
    PackerEngine::new().verify(&fixture.dist).unwrap();
    // Change only a valid ZIP comment: archive semantics and critical bytes stay
    // intact, so verification must detect the exact transport-byte change.
    let mut artifact = fs::read(&first.artifacts[0].path).unwrap();
    let end = artifact.len() - 22;
    assert_eq!(&artifact[end..end + 4], b"PK\x05\x06");
    let comment_length = artifact.len() - 2;
    artifact[comment_length..].copy_from_slice(&1_u16.to_le_bytes());
    artifact.push(b'!');
    fs::write(&first.artifacts[0].path, artifact).unwrap();
    assert_eq!(
        PackerEngine::new().verify(&fixture.dist).unwrap_err().code,
        ErrorCode::TransportDigestMismatch
    );
}

#[test]
fn invalid_critical_paths_missing_files_and_invalid_templates_preserve_published_index() {
    let mut fixture = Fixture::new(false);
    fixture.build().unwrap();
    let published = fs::read(fixture.dist.join("source-index.json")).unwrap();
    let valid = fixture.recipe.clone();
    fixture.recipe["games"][0]["layers"][1]["critical_files"][0]["path"] = json!("missing.exe");
    let error = fixture.build().unwrap_err();
    assert_eq!(error.code, ErrorCode::CorruptedCriticalBinary);
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    fixture.recipe = valid.clone();
    fixture.recipe["games"][0]["config_templates"][0]["template"] = json!("[unterminated");
    assert_eq!(
        fixture.build().unwrap_err().code,
        ErrorCode::InvalidTemplate
    );
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    fixture.recipe = valid.clone();
    fixture.recipe["games"][0]["layers"][1]["critical_files"][0]["path"] = json!("../outside.exe");
    assert_eq!(fixture.build().unwrap_err().code, ErrorCode::InvalidPath);
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    fixture.recipe = valid;
    fixture.recipe["games"][0]["id"] = json!("../escape");
    assert_eq!(fixture.build().unwrap_err().code, ErrorCode::InvalidId);
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    assert!(!fixture.temporary.path().join("escape.json").exists());
    PackerEngine::new().verify(&fixture.dist).unwrap();
}

#[test]
fn packer_refuses_archive_traversal_and_windows_case_collisions() {
    let mut fixture = Fixture::new(false);
    fixture.recipe["games"][0]["layers"][1]["critical_files"] = json!([]);
    fixture.recipe["games"][0]
        .as_object_mut()
        .unwrap()
        .remove("runtime");
    for names in [vec!["../escaped.bin"], vec!["main.exe", "MAIN.EXE"]] {
        let path = fixture.temporary.path().join("runtime.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for name in names {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"unsafe archive entry").unwrap();
        }
        writer.finish().unwrap();
        assert!(matches!(
            fixture.build().unwrap_err().code,
            ErrorCode::ArchiveInvalid | ErrorCode::InvalidPath
        ));
        assert!(!fixture.dist.join("source-index.json").exists());
        assert!(!fixture.temporary.path().join("escaped.bin").exists());
    }
}

#[test]
fn packer_self_check_resolves_parents_and_rejects_missing_or_cyclic_inheritance() {
    let mut fixture = Fixture::new(false);
    let parent = fixture.recipe["games"][0].clone();
    let child = json!({"id":"derived-game","title":"Derived fixture","version":"2.0", "extends":GAME_ID,"layers":[]});
    // The declaration order must not make a valid parent appear unavailable.
    fixture.recipe["games"] = json!([child, parent]);
    fixture.build().unwrap();
    PackerEngine::new().verify(&fixture.dist).unwrap();
    let parent: super::schema::GameManifest =
        serde_json::from_slice(&fs::read(fixture.dist.join("manifests/packer-game.json")).unwrap())
            .unwrap();
    let parents = BTreeMap::from([(
        QualifiedGameId {
            source_id: SOURCE_ID.into(),
            game_id: GAME_ID.into(),
        },
        parent,
    )]);
    let derived = super::evolution::SchemaUpcaster::upcast_game_manifest(
        SOURCE_ID,
        &fs::read_to_string(fixture.dist.join("manifests/derived-game.json")).unwrap(),
        &parents,
    )
    .unwrap();
    assert_eq!(derived.layers.len(), 3);
    assert_eq!(derived.runtime.as_ref().unwrap().layer_id, "runtime");
    let published = fs::read(fixture.dist.join("source-index.json")).unwrap();
    let mut coordinated_update = fixture.recipe.clone();
    coordinated_update["games"][0]["version"] = json!("2.1");
    coordinated_update["games"][1]["version"] = json!("1.0.1");
    fixture.recipe["games"] = json!([coordinated_update["games"][1].clone()]);
    assert!(matches!(
        fixture.build().unwrap_err().code,
        ErrorCode::MissingParent | ErrorCode::InvalidManifest
    ));
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    // Updating both entries refreshes the child's immutable ancestry binding.
    fixture.recipe = coordinated_update;
    fixture.build().unwrap();
    PackerEngine::new().verify(&fixture.dist).unwrap();
    let published = fs::read(fixture.dist.join("source-index.json")).unwrap();
    fixture.recipe["games"][0]["extends"] = json!("missing-parent");
    assert_eq!(fixture.build().unwrap_err().code, ErrorCode::MissingParent);
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    fixture.recipe["games"][0]["extends"] = json!(GAME_ID);
    fixture.recipe["games"][1]["extends"] = json!("derived-game");
    assert_eq!(fixture.build().unwrap_err().code, ErrorCode::ExtendsCycle);
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
}

#[tokio::test]
async fn packer_locator_keys_and_dynamic_attestations_feed_existing_resolvers() {
    use super::packer::{ArtifactInspectionOptions, SourceAttestedRelease};
    use super::plan::{ProviderCriticalFile, ProviderLockRequirement};
    use super::resolver::{
        DynamicAssetAttestation, DynamicProviderResolver, GithubReleaseClient,
        LocalLocatorResolver, ReleaseAsset, ReleaseMetadata,
    };
    use super::schema::{GameManifest, GithubReleaseProvider, LayerSource};

    let mut fixture = Fixture::new(false);
    let base = fixture.recipe["games"][0]["layers"][0]
        .as_object_mut()
        .unwrap();
    for field in ["local_path", "archive", "mirrors"] {
        base.remove(field);
    }
    base.insert(
        "local_locator".into(),
        json!({
            "reference_dir":"base","store":"manual",
            "path_hints":[fixture.temporary.path().join("base").to_string_lossy()],
            "key_files":[{"path":"data/core.dll","role":"core_library"}]
        }),
    );
    let authorization = json!({"release_id":"release-42","release_tag":"v1.0.0","asset_id":"asset-43",
        "asset_url":"https://github.com/owner/runtime/releases/download/v1.0.0/runtime-win.zip",
        "tool_versions":{"author":"packer-fixture"}});
    fixture.recipe["games"][0]["layers"][1]["dynamic_provider"] = json!({
        "repo":"owner/runtime","asset_mask":"runtime-*.zip","channel":"latest"
    });
    fixture.recipe["games"][0]["layers"][1]["attestation"] = authorization.clone();
    fixture.recipe["games"][0]["runtime"]["default_channel"] = json!("dynamic");
    let report = fixture.build().unwrap();
    assert_eq!(report.artifacts.len(), 2);
    assert_eq!(report.attestations.len(), 1);
    let verification = PackerEngine::new().verify(&fixture.dist).unwrap();
    assert_eq!(verification.attestations_verified, 1);
    let document: GameManifest =
        serde_json::from_slice(&fs::read(fixture.dist.join("manifests/packer-game.json")).unwrap())
            .unwrap();
    let base = document
        .layers
        .iter()
        .find(|layer| layer.id == "base")
        .unwrap();
    let LayerSource::LocalLocator { locator } = &base.source else {
        panic!("Expected generated local locator")
    };
    let key = &locator.key_files[0];
    assert_eq!(
        key.blake3,
        blake3::hash(b"test core library").to_hex().as_str()
    );
    let inventory = &report.locator_files["packer-game::base"];
    let file = inventory.iter().find(|file| file.path == key.path).unwrap();
    assert_eq!(file.size, b"test core library".len() as u64);
    assert_eq!(file.blake3, key.blake3);
    fs::create_dir_all(&fixture.library).unwrap();
    let located = LocalLocatorResolver::new()
        .resolve(locator, &fixture.library, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(located.root, fixture.temporary.path().join("base"));

    let options: ArtifactInspectionOptions = serde_json::from_value(json!({
        "archive":"zip","auto_main_executable":true,"attestation":authorization
    }))
    .unwrap();
    let inspection = PackerEngine::new()
        .inspect_artifact(&fixture.temporary.path().join("runtime.zip"), &options)
        .unwrap();
    let published: SourceAttestedRelease =
        serde_json::from_slice(&fs::read(&report.attestations[0]).unwrap()).unwrap();
    assert_eq!(inspection.attestation.as_ref(), Some(&published));
    assert_eq!(
        inspection.size_bytes,
        fs::metadata(fixture.temporary.path().join("runtime.zip"))
            .unwrap()
            .len()
    );
    let runtime = document
        .layers
        .iter()
        .find(|layer| layer.id == "runtime")
        .unwrap();
    assert_eq!(inspection.critical_files, runtime.critical_files);
    assert_eq!(
        Some(&inspection.transport_digest),
        runtime.transport_digest.as_ref()
    );

    struct AuthorizedRelease(ReleaseMetadata);
    #[async_trait]
    impl GithubReleaseClient for AuthorizedRelease {
        async fn release(
            &self,
            _: &GithubReleaseProvider,
            cancel: &Cancellation,
        ) -> Result<ReleaseMetadata, FusionError> {
            cancel.check()?;
            Ok(self.0.clone())
        }
    }
    let metadata = ReleaseMetadata {
        id: published.release_id.clone(),
        tag: published.tag.clone(),
        draft: false,
        prerelease: false,
        assets: vec![ReleaseAsset {
            id: published.asset_id.clone(),
            name: "runtime-win.zip".into(),
            url: published.asset_url.clone(),
            api_digest: None,
        }],
    };
    let expected: DynamicAssetAttestation = published.clone().into();
    let requirement = ProviderLockRequirement {
        release_and_asset: true,
        transport_digest: true,
        critical_files: runtime
            .critical_files
            .iter()
            .map(|file| ProviderCriticalFile {
                path: file.path.clone(),
                role: file.role,
            })
            .collect(),
    };
    let resolved = DynamicProviderResolver::new(Arc::new(AuthorizedRelease(metadata)))
        .resolve(
            runtime.dynamic_provider.as_ref().unwrap(),
            &requirement,
            Some(&expected),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        resolved.snapshot_lock().critical_files,
        inspection.critical_files
    );
    assert_eq!(
        resolved.snapshot_lock().transport_digest,
        inspection.transport_digest
    );
    assert_eq!(
        resolved.snapshot_lock().asset_url.as_deref(),
        Some(published.asset_url.as_str())
    );
    let published_index = fs::read(fixture.dist.join("source-index.json")).unwrap();
    fixture.recipe["games"][0]["layers"][1]["dynamic_provider"]["tag"] = json!("v2.0.0");
    assert_eq!(
        fixture.build().unwrap_err().code,
        ErrorCode::ProviderResolutionFailed
    );
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published_index
    );
    let mut tampered = published;
    tampered.release_id = "unauthorized-release".into();
    fs::write(
        &report.attestations[0],
        serde_json::to_vec(&tampered).unwrap(),
    )
    .unwrap();
    assert_eq!(
        PackerEngine::new().verify(&fixture.dist).unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
}

#[test]
fn packer_authors_patch_digests_and_final_critical_output_without_running_a_tool() {
    use super::schema::{DigestAlgorithm, GameManifest};

    let mut fixture = Fixture::new(false);
    let original = b"owned test game data";
    let patched = b"owned patched core library";
    let payload = b"deliberately not an executable xdelta stream";
    fs::write(fixture.temporary.path().join("delta.patch"), payload).unwrap();
    fs::write(fixture.temporary.path().join("patched.dll"), patched).unwrap();
    fixture.recipe["target_capabilities"] = json!(["patch.xdelta3.v1", "patch.bsdiff.v1"]);
    fixture.recipe["games"][0]["layers"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"delta","kind":"patch","depends_on":["base"],
            "local_path":"delta.patch","archive":"raw","mount_path":"data",
            "mirrors":["https://author.example/artifacts/{{ARTIFACT_FILENAME}}"],
            "critical_files":[{"path":"patched.dll","role":"core_library"}],
            "patch":{"format":"xdelta3","base_layer":"base",
                "input_path":"data/content.bin","output_path":"data/patched.dll",
                "input_reference":"base/data/content.bin","output_reference":"patched.dll"}
        }));
    let valid = fixture.recipe.clone();
    let report = fixture.build().unwrap();
    PackerEngine::new().verify(&fixture.dist).unwrap();
    let initial_index = fs::read(fixture.dist.join("source-index.json")).unwrap();
    fixture.recipe["target_capabilities"] =
        json!(["patch.bsdiff.v1", "patch.xdelta3.v1", "patch.xdelta3.v1"]);
    fixture.build().unwrap();
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        initial_index
    );
    fixture.recipe = valid.clone();
    let document: GameManifest =
        serde_json::from_slice(&fs::read(fixture.dist.join("manifests/packer-game.json")).unwrap())
            .unwrap();
    let layer = document
        .layers
        .iter()
        .find(|layer| layer.id == "delta")
        .unwrap();
    let patch = layer.patch.as_ref().unwrap();
    assert_eq!(patch.input_digest.algorithm, DigestAlgorithm::Blake3);
    assert_eq!(patch.output_digest.algorithm, DigestAlgorithm::Blake3);
    assert_eq!(
        patch.input_digest.value,
        blake3::hash(original).to_hex().as_str()
    );
    assert_eq!(
        patch.output_digest.value,
        blake3::hash(patched).to_hex().as_str()
    );
    assert_eq!(layer.critical_files.len(), 1);
    let critical = &layer.critical_files[0];
    assert_eq!(
        super::evolution::mounted_path(&layer.mount_path, &critical.path),
        patch.output_path
    );
    assert_eq!(critical.blake3, patch.output_digest.value);
    let artifact = report
        .artifacts
        .iter()
        .find(|artifact| artifact.layer_id == "delta")
        .unwrap();
    assert_eq!(artifact.raw_path.as_deref(), Some("patch.payload"));
    assert_eq!(
        artifact.transport_digest.value,
        blake3::hash(payload).to_hex().as_str()
    );
    assert_ne!(critical.blake3, artifact.transport_digest.value);
    // Both build and verify accept an inert placeholder payload: authoring hashes
    // the references and never executes or downloads an external patch tool.
    let published = fs::read(fixture.dist.join("source-index.json")).unwrap();
    fs::write(
        fixture.temporary.path().join("wrong-original.bin"),
        b"contradictory input reference",
    )
    .unwrap();
    fixture.recipe["games"][0]["layers"][3]["patch"]["input_reference"] =
        json!("wrong-original.bin");
    assert_eq!(
        fixture.build().unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    fixture.recipe = valid.clone();
    fixture.recipe["target_capabilities"] = json!([]);
    assert_eq!(
        fixture.build().unwrap_err().code,
        ErrorCode::MissingCapability
    );
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    fixture.recipe = valid;
    fixture.recipe["games"][0]["layers"][3]["critical_files"][0]["path"] = json!("unrelated.dll");
    assert_eq!(
        fixture.build().unwrap_err().code,
        ErrorCode::InvalidManifest
    );
    assert_eq!(
        fs::read(fixture.dist.join("source-index.json")).unwrap(),
        published
    );
    // Unicode case conversion can change byte lengths; malformed mount/output
    // combinations must return a typed error rather than panic while slicing.
    fixture.recipe["games"][0]["layers"][3]["mount_path"] = json!("K");
    fixture.recipe["games"][0]["layers"][3]["critical_files"] = json!([]);
    fixture.recipe["games"][0]["layers"][3]["auto_main_executable"] = json!(true);
    fixture.recipe["games"][0]["layers"][3]["patch"]["output_path"] = json!("K/main.exe");
    assert_eq!(
        fixture.build().unwrap_err().code,
        ErrorCode::InvalidManifest
    );
    PackerEngine::new().verify(&fixture.dist).unwrap();
}
