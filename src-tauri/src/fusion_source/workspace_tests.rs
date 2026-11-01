//! Local-only Stage 2 integration tests with real CAS trees and workspace leases.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde_json::{json, Value};

use super::canonical::{CanonicalGameManifest, CasAccess, QualifiedGameId};
use super::capabilities::CapabilityRegistry;
use super::cas::{CasObject, CasStore};
use super::config_templates::TemplateContext;
use super::errors::ErrorCode;
use super::evolution::SchemaUpcaster;
use super::fs_safety;
use super::layers::LayerInput;
use super::persistence::PersistenceRoots;
use super::plan::{EnvStep, ExecutionPlan, PlanBuilder, PlanOptions, PlanStep, TransformStep};
use super::workspace::{AssembledWorkspace, WorkspaceAssembler, WorkspaceBuildOptions};

const BASE: &[u8] = b"immutable game data";
const RUNTIME_ONE: &[u8] = b"runtime version one";
const RUNTIME_TWO: &[u8] = b"runtime version two";

struct Fixture {
    temporary: tempfile::TempDir,
    assembler: WorkspaceAssembler,
    cas: CasStore,
    base: CasObject,
    manifest: CanonicalGameManifest,
    inputs: BTreeMap<String, LayerInput>,
    context: TemplateContext,
    roots: PersistenceRoots,
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn registry() -> CapabilityRegistry {
    // Processes and network resolution are descriptive here. Stage 2 never
    // launches a process or contacts a provider during these local tests.
    CapabilityRegistry::new([
        "core.schema.v1",
        "core.plan.v1",
        "core.layers.v1",
        "vfs.hardlink.v1",
        "local_locator.v1",
        "patch.xdelta3.v1",
        "patch.bsdiff.v1",
        "proc.supervisor.v1",
        "core.config_templates.v1",
        "persistence.saves.v1",
    ])
}

fn document(runtime_bytes: &[u8], version: &str) -> Value {
    json!({
        "schema_version":"1.0", "id":"native-game", "title":"Local Stage 2 fixture",
        "version":version,"platform":"pc",
        "layers":[
            {"id":"base","kind":"base","source":{"kind":"user_provided"},
             "writable_copy_paths":["data/config/state.ini"]},
            {"id":"runtime","kind":"runtime","depends_on":["base"],
             "source":{"kind":"user_provided"},"mount_path":"runtime",
             "critical_files":[{"path":"main.exe","blake3":hash(runtime_bytes),"role":"main_executable"}]}
        ],
        "runtime":{"layer_id":"runtime","executable":"runtime/main.exe"},
        "persistence":{"saves_paths":["workspace/saves/progress.dat"]},
        "config_templates":[{"id":"settings","path":"data/config/generated.ini","format":"ini",
            "template":"[Display]\nwidth={{DISPLAY_WIDTH}}\nheight={{DISPLAY_HEIGHT}}\n"}]
    })
}

fn canonical(document: &Value) -> CanonicalGameManifest {
    SchemaUpcaster::upcast_game_manifest("test-source", &document.to_string(), &BTreeMap::new())
        .unwrap()
}

fn plan(manifest: &CanonicalGameManifest) -> ExecutionPlan {
    PlanBuilder::build(manifest, &PlanOptions::default(), &registry()).unwrap()
}

fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let library = temporary.path().join("library");
    let input = temporary.path().join("base-input");
    let runtime = temporary.path().join("runtime-input");
    let appdata = temporary.path().join("original-appdata");
    let local_appdata = temporary.path().join("original-local-appdata");
    for path in [
        &library,
        &input.join("data/config"),
        &input.join("saves"),
        &runtime,
        &appdata,
        &local_appdata,
    ] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(input.join("data/static.bin"), BASE).unwrap();
    fs::write(
        input.join("data/config/state.ini"),
        b"private state original",
    )
    .unwrap();
    fs::write(
        input.join("data/config/generated.ini"),
        b"CAS template original",
    )
    .unwrap();
    fs::write(input.join("saves/progress.dat"), b"initial save").unwrap();
    fs::write(runtime.join("main.exe"), RUNTIME_ONE).unwrap();
    let cas = CasStore::open(&library).unwrap();
    let base = cas.import_directory(&input).unwrap();
    let runtime = cas.import_directory(&runtime).unwrap();
    let inputs = BTreeMap::from([
        (
            "base".into(),
            LayerInput::Cas {
                digest: base.digest.clone(),
                runtime_lock: None,
            },
        ),
        (
            "runtime".into(),
            LayerInput::Cas {
                digest: runtime.digest,
                runtime_lock: None,
            },
        ),
    ]);
    let assembler = WorkspaceAssembler::open(
        &library,
        &QualifiedGameId {
            source_id: "test-source".into(),
            game_id: "native-game".into(),
        },
    )
    .unwrap();
    let context = TemplateContext {
        workspace_dir: library,
        appdata_dir: appdata.clone(),
        local_appdata_dir: local_appdata.clone(),
        display_width: 2560,
        display_height: 1440,
        refresh_rate: 120,
        enabled_addons: BTreeSet::new(),
    };
    Fixture {
        temporary,
        assembler,
        cas,
        base,
        manifest: canonical(&document(RUNTIME_ONE, "1")),
        inputs,
        context,
        roots: PersistenceRoots {
            appdata,
            local_appdata,
        },
    }
}

impl Fixture {
    fn assemble(
        &self,
        manifest: &CanonicalGameManifest,
        inputs: &BTreeMap<String, LayerInput>,
    ) -> Result<AssembledWorkspace, super::errors::FusionError> {
        self.assembler.assemble(
            manifest,
            &plan(manifest),
            inputs,
            &self.context,
            &self.roots,
            &WorkspaceBuildOptions::default(),
        )
    }

    fn newer(
        &self,
        bytes: &[u8],
        version: &str,
    ) -> (CanonicalGameManifest, BTreeMap<String, LayerInput>) {
        let source = self.temporary.path().join(format!("runtime-{version}"));
        fs::create_dir(&source).unwrap();
        fs::write(source.join("main.exe"), bytes).unwrap();
        let runtime = self.cas.import_directory(&source).unwrap();
        let mut inputs = self.inputs.clone();
        inputs.insert(
            "runtime".into(),
            LayerInput::Cas {
                digest: runtime.digest,
                runtime_lock: None,
            },
        );
        (canonical(&document(bytes, version)), inputs)
    }
}

#[test]
fn legacy_projection_later_overlay_wins_in_workspace() {
    let f = fixture();
    let source = f.temporary.path().join("overlay");
    fs::create_dir_all(source.join("data")).unwrap();
    fs::write(source.join("data/static.bin"), b"overlay wins").unwrap();
    let object = f.cas.import_directory(&source).unwrap();
    let mut doc = document(RUNTIME_ONE, "1");
    doc["layers"].as_array_mut().unwrap().push(json!({
        "id":"overlay", "kind":"mod", "depends_on":["base"],
        "source":{"kind":"user_provided"}
    }));
    let mut inputs = f.inputs.clone();
    inputs.insert(
        "overlay".into(),
        LayerInput::Cas {
            digest: object.digest,
            runtime_lock: None,
        },
    );
    let assembled = f.assemble(&canonical(&doc), &inputs).unwrap();
    assert_eq!(
        fs::read(assembled.snapshot.workspace_root.join("data/static.bin")).unwrap(),
        b"overlay wins"
    );
    assert_eq!(fs::read(f.base.root.join("data/static.bin")).unwrap(), BASE);
}

#[test]
fn legacy_projection_parent_conflict_is_rejected_before_candidate_creation() {
    let f = fixture();
    let source = f.temporary.path().join("parent-conflict");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("data"), b"not a directory").unwrap();
    let object = f.cas.import_directory(&source).unwrap();
    let mut doc = document(RUNTIME_ONE, "1");
    doc["layers"].as_array_mut().unwrap().push(json!({
        "id":"overlay", "kind":"mod", "depends_on":["base"],
        "source":{"kind":"user_provided"}
    }));
    let mut inputs = f.inputs.clone();
    inputs.insert(
        "overlay".into(),
        LayerInput::Cas {
            digest: object.digest,
            runtime_lock: None,
        },
    );
    assert_eq!(
        f.assemble(&canonical(&doc), &inputs).unwrap_err().code,
        ErrorCode::InvalidManifest
    );
    let snapshots = f
        .context
        .workspace_dir
        .join(".fusion-source/workspaces/test-source/native-game/snapshots");
    assert_eq!(fs::read_dir(snapshots).unwrap().count(), 0);
}

#[test]
fn immutable_projection_uses_hardlinks_and_mutable_outputs_preserve_cas() {
    let f = fixture();
    let assembled = f.assemble(&f.manifest, &f.inputs).unwrap();
    let root = &assembled.snapshot.workspace_root;
    let immutable = f.base.root.join("data/static.bin");
    assert_eq!(
        fs_safety::file_identity(&immutable).unwrap(),
        fs_safety::file_identity(&root.join("data/static.bin")).unwrap()
    );
    assert!(fs::metadata(&immutable).unwrap().permissions().readonly());
    let mutable = f.base.root.join("data/config/state.ini");
    assert_ne!(
        fs_safety::file_identity(&mutable).unwrap(),
        fs_safety::file_identity(&root.join("data/config/state.ini")).unwrap()
    );
    fs::write(root.join("data/config/state.ini"), b"workspace mutation").unwrap();
    assert_eq!(fs::read(&mutable).unwrap(), b"private state original");
    assert_eq!(
        fs::read(f.base.root.join("data/config/generated.ini")).unwrap(),
        b"CAS template original"
    );
    assert!(fs::read_to_string(root.join("data/config/generated.ini"))
        .unwrap()
        .contains("width=2560"));
    assert!(!fs::metadata(root.join("data/config/generated.ini"))
        .unwrap()
        .permissions()
        .readonly());
}

#[test]
fn update_reuses_base_backs_up_latest_atomic_save_and_rollback_keeps_saves() {
    let f = fixture();
    let first = f.assemble(&f.manifest, &f.inputs).unwrap();
    let first_id = first.snapshot.id.clone();
    fs_safety::atomic_write(
        &first.snapshot.workspace_root.join("saves/progress.dat"),
        b"save before update",
    )
    .unwrap();
    drop(first);
    let (manifest, inputs) = f.newer(RUNTIME_TWO, "2");
    let second = f.assemble(&manifest, &inputs).unwrap();
    assert_eq!(second.snapshot.delta.reused, vec!["base"]);
    assert_eq!(second.snapshot.delta.changed, vec!["runtime"]);
    let backup = second.snapshot.save_backup.as_ref().unwrap();
    assert_eq!(
        fs::read(backup.root.join("workspace/saves/progress.dat")).unwrap(),
        b"save before update"
    );
    assert_eq!(
        fs::read(second.snapshot.workspace_root.join("runtime/main.exe")).unwrap(),
        RUNTIME_TWO
    );
    fs_safety::atomic_write(
        &second.snapshot.workspace_root.join("saves/progress.dat"),
        b"save after update",
    )
    .unwrap();
    drop(second);
    let rolled_back = f.assembler.rollback().unwrap();
    assert_eq!(rolled_back.snapshot.id, first_id);
    assert_eq!(
        fs::read(rolled_back.snapshot.workspace_root.join("runtime/main.exe")).unwrap(),
        RUNTIME_ONE
    );
    assert_eq!(
        fs::read(
            rolled_back
                .snapshot
                .workspace_root
                .join("saves/progress.dat")
        )
        .unwrap(),
        b"save after update"
    );
}

#[test]
fn invalid_render_or_binary_hash_never_changes_the_active_snapshot() {
    let f = fixture();
    let first = f.assemble(&f.manifest, &f.inputs).unwrap();
    let active = first.snapshot.id.clone();
    drop(first);
    let (mut manifest, inputs) = f.newer(RUNTIME_TWO, "2");
    manifest.config_templates[0].template = "invalid ini entry".into();
    assert_eq!(
        f.assemble(&manifest, &inputs).unwrap_err().code,
        ErrorCode::InvalidTemplate
    );
    assert_eq!(f.assembler.active().unwrap().unwrap().snapshot.id, active);
    let mut document = document(RUNTIME_TWO, "2");
    document["layers"][1]["critical_files"][0]["blake3"] = json!(hash(b"wrong binary"));
    let manifest = canonical(&document);
    assert_eq!(
        f.assemble(&manifest, &inputs).unwrap_err().code,
        ErrorCode::CorruptedCriticalBinary
    );
    assert_eq!(f.assembler.active().unwrap().unwrap().snapshot.id, active);
}

#[test]
fn a_live_workspace_lease_blocks_updates_and_rollback() {
    let f = fixture();
    let lease = f.assemble(&f.manifest, &f.inputs).unwrap();
    assert_eq!(
        f.assemble(&f.manifest, &f.inputs).unwrap_err().code,
        ErrorCode::OperationInProgress
    );
    assert_eq!(
        f.assembler.rollback().unwrap_err().code,
        ErrorCode::OperationInProgress
    );
    drop(lease);
    assert!(f.assembler.active().unwrap().is_some());
}

#[test]
fn a_patch_without_a_pinned_tool_preserves_the_active_workspace() {
    let f = fixture();
    let first = f.assemble(&f.manifest, &f.inputs).unwrap();
    let active = first.snapshot.id.clone();
    drop(first);
    let mut document = document(RUNTIME_ONE, "2");
    document["layers"].as_array_mut().unwrap().push(json!({
        "id":"delta","kind":"patch","source":{"kind":"user_provided"},
        "patch":{"format":"xdelta3","base_layer":"base",
            "input_path":"data/static.bin","output_path":"data/patched.bin",
            "input_digest":{"algorithm":"blake3","value":hash(BASE)},
            "output_digest":{"algorithm":"blake3","value":hash(b"patched data")}}
    }));
    let manifest = canonical(&document);
    let patch_file = f.temporary.path().join("patch.payload");
    fs::write(&patch_file, b"test patch payload").unwrap();
    let patch = f.cas.import_file(&patch_file, "patch.payload").unwrap();
    let mut inputs = f.inputs.clone();
    inputs.insert(
        "delta".into(),
        LayerInput::Cas {
            digest: patch.digest,
            runtime_lock: None,
        },
    );
    let options = WorkspaceBuildOptions::default();
    let error = f
        .assembler
        .assemble(
            &manifest,
            &plan(&manifest),
            &inputs,
            &f.context,
            &f.roots,
            &options,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MissingPatchTool);
    assert_eq!(f.assembler.active().unwrap().unwrap().snapshot.id, active);
}

#[test]
fn patch_barrier_path_conflicts_are_rejected_before_persistence_and_candidate_mutation() {
    for (output, later) in [
        ("data/patched.bin", "data/static.bin/child.bin"),
        ("data/patched.bin", "data"),
        ("data/patched.bin", "data/patched.bin/child.bin"),
        ("data/nested/patched.bin", "data/nested"),
        ("data/static.bin/child.bin", "unrelated.bin"),
        ("data", "unrelated.bin"),
    ] {
        let f = fixture();
        let first = f.assemble(&f.manifest, &f.inputs).unwrap();
        let active = first.snapshot.id.clone();
        fs_safety::atomic_write(
            &first.snapshot.workspace_root.join("saves/progress.dat"),
            b"live save not yet reconciled",
        )
        .unwrap();
        drop(first);
        let persistence = f
            .context
            .workspace_dir
            .join(".fusion-source/persistence/test-source/native-game");
        let fingerprint = || {
            fs_safety::list_plain_files(&persistence)
                .unwrap()
                .into_iter()
                .map(|(path, file)| (path, fs::read(file).unwrap()))
                .collect::<Vec<_>>()
        };
        let before = fingerprint();
        let source = f.temporary.path().join("post-patch");
        let file = source.join(later);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"later ordinary file").unwrap();
        let object = f.cas.import_directory(&source).unwrap();
        let payload = f.temporary.path().join("patch.payload");
        fs::write(&payload, b"not executed by preflight").unwrap();
        let patch = f.cas.import_file(&payload, "patch.payload").unwrap();
        let mut inputs = f.inputs.clone();
        for (id, digest) in [("delta", patch.digest), ("post", object.digest)] {
            inputs.insert(
                id.into(),
                LayerInput::Cas {
                    digest,
                    runtime_lock: None,
                },
            );
        }
        let mut doc = document(RUNTIME_ONE, "2");
        doc["layers"].as_array_mut().unwrap().extend([
            json!({"id":"delta","kind":"patch","source":{"kind":"user_provided"},
                "patch":{"format":"xdelta3","base_layer":"base",
                    "input_path":"data/static.bin","output_path":output,
                    "input_digest":{"algorithm":"blake3","value":hash(BASE)},
                    "output_digest":{"algorithm":"blake3","value":hash(b"patched data")}}}),
            json!({"id":"post","kind":"mod","depends_on":["delta"],
                "source":{"kind":"user_provided"}}),
        ]);
        assert_eq!(
            f.assemble(&canonical(&doc), &inputs).unwrap_err().code,
            ErrorCode::InvalidManifest,
            "output={output}, later={later}"
        );
        assert_eq!(
            fingerprint(),
            before,
            "persistence changed for {output}, {later}"
        );
        assert_eq!(f.assembler.active().unwrap().unwrap().snapshot.id, active);
        let snapshots = f
            .context
            .workspace_dir
            .join(".fusion-source/workspaces/test-source/native-game/snapshots");
        assert_eq!(fs::read_dir(snapshots).unwrap().count(), 1);
    }
}

#[test]
fn local_locator_lease_preserves_original_permissions_and_rechecks_key_hashes() {
    let f = fixture();
    let local = f.temporary.path().join("base-input");
    let mut document = document(RUNTIME_ONE, "1");
    document["layers"][0]["source"] = json!({"kind":"local_locator","locator":{"store":"manual","path_hints":[local.to_string_lossy()],"key_files":[{"path":"data/static.bin","blake3":hash(BASE),"role":"core_library"}]}});
    let manifest = canonical(&document);
    let mut inputs = f.inputs.clone();
    inputs.insert(
        "base".into(),
        LayerInput::LocalDirectory {
            root: local.clone(),
        },
    );
    let before = fs::metadata(local.join("data/static.bin"))
        .unwrap()
        .permissions()
        .readonly();
    let lease = f.assemble(&manifest, &inputs).unwrap();
    assert_eq!(
        fs::metadata(local.join("data/static.bin"))
            .unwrap()
            .permissions()
            .readonly(),
        before
    );
    #[cfg(windows)]
    {
        assert!(fs::write(local.join("data/static.bin"), b"forbidden").is_err());
        assert!(fs::write(
            lease.snapshot.workspace_root.join("data/static.bin"),
            b"forbidden"
        )
        .is_err());
    }
    drop(lease);
    fs::write(local.join("data/static.bin"), b"storefront changed").unwrap();
    assert!(f.assembler.active().is_err());
    assert_eq!(
        fs::read(local.join("data/static.bin")).unwrap(),
        b"storefront changed"
    );
    assert_eq!(
        fs::metadata(local.join("data/static.bin"))
            .unwrap()
            .permissions()
            .readonly(),
        before
    );
}

#[test]
fn cleanup_preserves_unrecorded_files_and_never_deletes_persistent_saves() {
    let f = fixture();
    let first = f.assemble(&f.manifest, &f.inputs).unwrap();
    let first_id = first.snapshot.id.clone();
    let first_root = first.snapshot.workspace_root.clone();
    assert!(f.assembler.cleanup_snapshot(&first_id).is_err());
    drop(first);
    let (second_manifest, second_inputs) = f.newer(RUNTIME_TWO, "2");
    let second = f.assemble(&second_manifest, &second_inputs).unwrap();
    let second_id = second.snapshot.id.clone();
    drop(second);
    assert!(f.assembler.cleanup_snapshot(&first_id).is_err());
    let (third_manifest, third_inputs) = f.newer(b"runtime version three", "3");
    let third = f.assemble(&third_manifest, &third_inputs).unwrap();
    let save = third.snapshot.persistence.bindings[0]
        .persistent_path
        .clone();
    drop(third);
    assert!(f.assembler.cleanup_snapshot(&second_id).is_err());
    let sentinel = first_root.join("unknown-sentinel");
    fs::write(&sentinel, b"preserve outside ownership").unwrap();
    assert!(f.assembler.cleanup_snapshot(&first_id).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve outside ownership");
    fs::remove_file(&sentinel).unwrap();
    f.assembler.cleanup_snapshot(&first_id).unwrap();
    assert!(!first_root.exists());
    assert_eq!(fs::read(save).unwrap(), b"initial save");
    assert!(f.assembler.active().unwrap().is_some());
}

#[test]
fn cleanup_rejects_a_directory_save_binding_replaced_with_an_unowned_tree() {
    let f = fixture();
    let mut first_manifest = f.manifest.clone();
    first_manifest.persistence.saves_paths = vec!["workspace/saves".into()];
    let first = f.assemble(&first_manifest, &f.inputs).unwrap();
    let first_id = first.snapshot.id.clone();
    let first_root = first.snapshot.workspace_root.clone();
    let persistent = first.snapshot.persistence.bindings[0]
        .persistent_path
        .clone();
    drop(first);
    let (mut second_manifest, second_inputs) = f.newer(RUNTIME_TWO, "2");
    second_manifest.persistence.saves_paths = vec!["workspace/saves".into()];
    drop(f.assemble(&second_manifest, &second_inputs).unwrap());
    let (mut third_manifest, third_inputs) = f.newer(b"runtime version three", "3");
    third_manifest.persistence.saves_paths = vec!["workspace/saves".into()];
    drop(f.assemble(&third_manifest, &third_inputs).unwrap());
    let replaced = first_root.join("saves");
    super::persistence::unlink_workspace_save_link(&replaced).unwrap();
    fs::create_dir(&replaced).unwrap();
    let sentinel = replaced.join("unknown-sentinel");
    fs::write(&sentinel, b"preserve unowned save replacement").unwrap();
    assert!(f.assembler.cleanup_snapshot(&first_id).is_err());
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"preserve unowned save replacement"
    );
    assert_eq!(
        fs::read(persistent.join("progress.dat")).unwrap(),
        b"initial save"
    );
    assert!(first_root.join("runtime/main.exe").exists());
}

#[test]
fn a_mutated_plan_cannot_skip_configs_change_mounts_or_add_an_overlay() {
    let f = fixture();
    let first = f.assemble(&f.manifest, &f.inputs).unwrap();
    let active = first.snapshot.id.clone();
    drop(first);
    let original = plan(&f.manifest);
    let mut missing_config = original.clone();
    missing_config
        .steps
        .retain(|step| !matches!(step, PlanStep::Env(EnvStep::RenderConfigTemplate { .. })));
    let mut changed_mount = original.clone();
    let project = changed_mount
        .steps
        .iter_mut()
        .find_map(|step| match step {
            PlanStep::Transform(TransformStep::ProjectLayer { mount_path, .. }) => Some(mount_path),
            _ => None,
        })
        .unwrap();
    *project = "undeclared-location".into();
    for mutated in [missing_config, changed_mount] {
        let error = f
            .assembler
            .assemble(
                &f.manifest,
                &mutated,
                &f.inputs,
                &f.context,
                &f.roots,
                &WorkspaceBuildOptions::default(),
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidManifest);
        assert_eq!(f.assembler.active().unwrap().unwrap().snapshot.id, active);
    }
    let mut overlay = original;
    overlay
        .steps
        .push(PlanStep::Transform(TransformStep::MountOverlay {
            layer_ids: vec!["base".into()],
            cas_access: CasAccess::ReadOnly,
        }));
    let error = f
        .assembler
        .assemble(
            &f.manifest,
            &overlay,
            &f.inputs,
            &f.context,
            &f.roots,
            &WorkspaceBuildOptions::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MissingCapability);
    assert_eq!(f.assembler.active().unwrap().unwrap().snapshot.id, active);
}

#[cfg(windows)]
#[test]
fn actual_cross_volume_locator_requires_an_explicit_available_link_strategy() {
    let f = fixture();
    // Both write locations are explicit task workspace/temp roots. No storefront
    // installation or another user folder is created or modified by this test.
    let task_temp = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has a repository parent")
        .join(".tmp");
    fs::create_dir_all(&task_temp).unwrap();
    let external = tempfile::tempdir_in(&task_temp).unwrap();
    let local = external.path().join("owned-base");
    fs::create_dir_all(local.join("data/config")).unwrap();
    fs::create_dir(local.join("saves")).unwrap();
    fs::write(local.join("data/static.bin"), BASE).unwrap();
    fs::write(
        local.join("data/config/state.ini"),
        b"external state original",
    )
    .unwrap();
    fs::write(
        local.join("data/config/generated.ini"),
        b"external config original",
    )
    .unwrap();
    fs::write(local.join("saves/progress.dat"), b"external save original").unwrap();
    if fs_safety::same_volume(&local, &f.context.workspace_dir).unwrap() {
        eprintln!("Cross-volume workspace regression requires the repository and system temporary directory on different volumes; no distinct volume is available on this host.");
        return;
    }
    fs::create_dir(f.roots.appdata.join("Game")).unwrap();
    fs::write(
        f.roots.appdata.join("Game/profile.dat"),
        b"global profile original",
    )
    .unwrap();
    let mut document = document(RUNTIME_ONE, "1");
    document["layers"][0]["source"] = json!({"kind":"local_locator","locator":{"store":"manual","path_hints":[local.to_string_lossy()],"key_files":[{"path":"data/static.bin","blake3":hash(BASE),"role":"core_library"}]}});
    document["persistence"]["saves_paths"]
        .as_array_mut()
        .unwrap()
        .push(json!("{{APPDATA}}/Game"));
    document["config_templates"].as_array_mut().unwrap().push(json!({"id":"environment","path":"data/config/env.json","format":"json","template":"{\"save\":\"{{APPDATA}}/Game\"}"}));
    let mut inputs = f.inputs.clone();
    inputs.insert(
        "base".into(),
        LayerInput::LocalDirectory {
            root: local.clone(),
        },
    );
    let manifest = canonical(&document);
    assert_eq!(
        f.assemble(&manifest, &inputs).unwrap_err().code,
        ErrorCode::CrossVolumeLinkNotSupported
    );
    assert!(f.assembler.active().unwrap().is_none());
    document["local_placement"] = json!("cross_volume_fallback");
    let manifest = canonical(&document);
    match f.assemble(&manifest, &inputs) {
        Ok(lease) => {
            let root = &lease.snapshot.workspace_root;
            assert!(fs::symlink_metadata(root.join("data/static.bin"))
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read(root.join("data/static.bin")).unwrap(), BASE);
            fs::write(root.join("data/config/state.ini"), b"private state changed").unwrap();
            fs::write(root.join("saves/progress.dat"), b"private saves changed").unwrap();
            let appdata =
                std::path::PathBuf::from(&lease.snapshot.persistence.env_overrides["APPDATA"]);
            assert_ne!(appdata, f.roots.appdata);
            fs::write(appdata.join("Game/profile.dat"), b"private profile changed").unwrap();
            let environment: Value =
                serde_json::from_slice(&fs::read(root.join("data/config/env.json")).unwrap())
                    .unwrap();
            assert_eq!(environment["save"], format!("{}/Game", appdata.display()));
            assert_eq!(
                fs::read(local.join("data/config/state.ini")).unwrap(),
                b"external state original"
            );
            assert_eq!(
                fs::read(local.join("data/config/generated.ini")).unwrap(),
                b"external config original"
            );
            assert_eq!(
                fs::read(local.join("saves/progress.dat")).unwrap(),
                b"external save original"
            );
            assert_eq!(
                fs::read(f.roots.appdata.join("Game/profile.dat")).unwrap(),
                b"global profile original"
            );
            assert!(!fs::metadata(local.join("data/static.bin"))
                .unwrap()
                .permissions()
                .readonly());
            let checked = super::integrity::PreLaunchIntegrityGuard::verify_and_lock(
                &lease.snapshot,
                &TestTrust(true),
            )
            .unwrap();
            // Lock the projected symlink leaf itself as well as the original
            // bytes; replacing that leaf must not redirect process path lookup.
            assert!(fs::remove_file(root.join("data/static.bin")).is_err());
            drop(checked);
        }
        Err(error) => {
            assert_eq!(error.code, ErrorCode::CrossVolumeLinkNotSupported);
            assert!(f.assembler.active().unwrap().is_none());
            assert_eq!(fs::read(local.join("data/static.bin")).unwrap(), BASE);
            assert_eq!(
                fs::read(local.join("saves/progress.dat")).unwrap(),
                b"external save original"
            );
        }
    }
}

struct TestTrust(bool);
impl super::integrity::ExecutableTrust for TestTrust {
    fn is_trusted(
        &self,
        _game: &QualifiedGameId,
        _path: &str,
        _blake3: &str,
    ) -> Result<bool, super::errors::FusionError> {
        Ok(self.0)
    }
}

#[test]
fn prelaunch_classifies_a_deleted_injector_without_confusing_container_integrity() {
    let f = fixture();
    let source = f.temporary.path().join("injected-runtime");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("main.exe"), RUNTIME_ONE).unwrap();
    fs::write(source.join("coop.dll"), b"verified injector").unwrap();
    let object = f.cas.import_directory(&source).unwrap();
    let mut inputs = f.inputs.clone();
    inputs.insert(
        "runtime".into(),
        LayerInput::Cas {
            digest: object.digest.clone(),
            runtime_lock: None,
        },
    );
    let mut document = document(RUNTIME_ONE, "1");
    document["layers"][1]["critical_files"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "path":"coop.dll", "blake3":hash(b"verified injector"), "role":"injector_dll"
        }));
    let manifest = canonical(&document);
    let lease = f.assemble(&manifest, &inputs).unwrap();
    super::integrity::PreLaunchIntegrityGuard::verify(&lease.snapshot, &TestTrust(true)).unwrap();
    fs_safety::unlink_file(&lease.snapshot.workspace_root.join("runtime/coop.dll")).unwrap();
    assert_eq!(
        super::integrity::PreLaunchIntegrityGuard::verify(&lease.snapshot, &TestTrust(true))
            .unwrap_err()
            .code,
        ErrorCode::MissingInjectorDll
    );
    drop(lease);
    assert_eq!(
        f.assembler.active().unwrap_err().code,
        ErrorCode::MissingInjectorDll
    );
    // The transport/CAS tree remains valid despite the missing projection.
    f.cas.verify(&object.digest).unwrap();
}

#[test]
fn prelaunch_requires_exact_trust_and_retains_verified_executable_handles() {
    let f = fixture();
    let mut document = document(RUNTIME_ONE, "1");
    document["layers"][1]["writable_copy_paths"] = json!(["main.exe"]);
    let manifest = canonical(&document);
    let lease = f.assemble(&manifest, &f.inputs).unwrap();
    assert_eq!(
        super::integrity::PreLaunchIntegrityGuard::verify(&lease.snapshot, &TestTrust(false))
            .unwrap_err()
            .code,
        ErrorCode::UntrustedExecutable
    );
    let handles = super::integrity::PreLaunchIntegrityGuard::verify_and_lock(
        &lease.snapshot,
        &TestTrust(true),
    )
    .unwrap();
    let executable = lease.snapshot.workspace_root.join("runtime/main.exe");
    #[cfg(windows)]
    {
        assert!(fs::write(&executable, b"changed while checked").is_err());
        let parent = executable.parent().unwrap();
        assert!(fs::rename(parent, parent.with_file_name("replaced-runtime")).is_err());
    }
    drop(handles);
    fs::write(&executable, b"a modified executable").unwrap();
    assert_eq!(
        super::integrity::PreLaunchIntegrityGuard::verify(&lease.snapshot, &TestTrust(true))
            .unwrap_err()
            .code,
        ErrorCode::CorruptedCriticalBinary
    );
}

#[test]
fn verified_manifest_binding_cannot_label_a_different_canonical_build() {
    let f = fixture();
    let binding = super::workspace::VerifiedManifestBinding::from_verified(
        &f.manifest,
        &hash(b"verified source manifest bytes"),
    )
    .unwrap();
    let mut other = f.manifest.clone();
    other.version = "2".into();
    let error = f
        .assembler
        .assemble(
            &other,
            &plan(&other),
            &f.inputs,
            &f.context,
            &f.roots,
            &WorkspaceBuildOptions {
                verified_manifest: Some(binding),
                ..WorkspaceBuildOptions::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ManifestDigestMismatch);
    assert!(f.assembler.active().unwrap().is_none());
}

#[test]
fn prelaunch_rejects_a_replaced_owned_workspace_directory() {
    let f = fixture();
    let lease = f.assemble(&f.manifest, &f.inputs).unwrap();
    let workspace = &lease.snapshot.workspace_root;
    let displaced = workspace.with_file_name("displaced-owned-workspace");
    fs::rename(workspace, &displaced).unwrap();
    fs::create_dir(workspace).unwrap();
    assert_eq!(
        super::integrity::PreLaunchIntegrityGuard::verify(&lease.snapshot, &TestTrust(true))
            .unwrap_err()
            .code,
        ErrorCode::IntegrityViolation
    );
}

#[test]
fn prelaunch_classifies_an_original_locator_key_changed_after_lease_release() {
    let f = fixture();
    let base = f.temporary.path().join("located-base-integrity");
    fs::create_dir_all(base.join("data")).unwrap();
    fs::write(base.join("data/static.bin"), BASE).unwrap();
    let mut document = document(RUNTIME_ONE, "1");
    document["layers"][0]["source"] = json!({"kind":"local_locator","locator":{
    "store":"manual","path_hints":[base.to_string_lossy()],"key_files":[{
        "path":"data/static.bin","blake3":hash(BASE),"role":"core_library"
    }]}});
    let manifest = canonical(&document);
    let mut inputs = f.inputs.clone();
    inputs.insert(
        "base".into(),
        LayerInput::LocalDirectory { root: base.clone() },
    );
    let lease = f.assemble(&manifest, &inputs).unwrap();
    let snapshot = lease.snapshot.clone();
    drop(lease);
    fs::write(base.join("data/static.bin"), b"Steam updated the base").unwrap();
    assert_eq!(
        super::integrity::PreLaunchIntegrityGuard::verify(&snapshot, &TestTrust(true))
            .unwrap_err()
            .code,
        ErrorCode::LocalBaseModified
    );
    assert_eq!(
        f.assembler.active().unwrap_err().code,
        ErrorCode::LocalBaseModified
    );
}

#[test]
fn cancelled_candidate_preserves_the_active_snapshot() {
    let f = fixture();
    let installed = f.assemble(&f.manifest, &f.inputs).unwrap();
    let id = installed.snapshot.id.clone();
    drop(installed);
    let (manifest, inputs) = f.newer(RUNTIME_TWO, "2");
    let cancel = super::adapters::Cancellation::default();
    cancel.cancel();
    let error = f
        .assembler
        .assemble(
            &manifest,
            &plan(&manifest),
            &inputs,
            &f.context,
            &f.roots,
            &WorkspaceBuildOptions {
                cancellation: Some(cancel),
                ..WorkspaceBuildOptions::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(f.assembler.active().unwrap().unwrap().snapshot.id, id);
}

#[test]
fn uninstall_preserves_unknown_files_persistent_saves_and_cas() {
    let f = fixture();
    let installed = f.assemble(&f.manifest, &f.inputs).unwrap();
    let id = installed.snapshot.id.clone();
    let unknown = installed.snapshot.workspace_root.join("user-note.txt");
    fs::write(&unknown, b"keep this user file").unwrap();
    fs::write(
        installed.snapshot.workspace_root.join("saves/progress.dat"),
        b"latest progress",
    )
    .unwrap();
    installed.reconcile_saves().unwrap();
    let saves = installed.snapshot.persistence.bindings[0]
        .persistent_path
        .clone();
    drop(installed);
    let retained = f.assembler.remove_installation().unwrap();
    assert!(f.assembler.active().unwrap().is_none());
    assert!(retained.contains(&id));
    assert_eq!(fs::read(unknown).unwrap(), b"keep this user file");
    assert_eq!(fs::read(saves).unwrap(), b"latest progress");
    f.cas.verify(&f.base.digest).unwrap();
}
