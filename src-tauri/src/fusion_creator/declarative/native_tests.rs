//! Native generic executor checks use the existing H-owned fixture environment.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::Arc;

use crate::fusion_source::cas::CasStore;
use crate::fusion_source::config_templates::TemplateContext;
use crate::fusion_source::integrity::ExecutableTrust;
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::persistence::PersistenceRoots;
use crate::fusion_source::supervisor::{ProcessSupervisor, SupervisorOptions};
use crate::fusion_source::test_support::CreatorHarness;
use crate::fusion_source::workspace::{
    WorkspaceAssembler, WorkspaceBuildOptions, WorkspaceSnapshot,
};
use crate::fusion_source::{
    CapabilityRegistry, ErrorCode, FusionError, PlanBuilder, PlanOptions, QualifiedGameId,
};

const HELPER: &str = "fusion_creator::declarative::native_tests::native_program_fixture";

#[test]
fn native_program_fixture() {
    let Some(root) = std::env::var_os("FUSION_DECLARATIVE_DATA") else {
        return;
    };
    let path = std::path::PathBuf::from(root).join("save.dat");
    let previous = fs::read_to_string(&path)
        .unwrap_or_default()
        .parse::<u32>()
        .unwrap_or(0);
    crate::fusion_source::fs_safety::atomic_write(&path, (previous + 1).to_string().as_bytes())
        .unwrap();
    println!("generic declarative process completed");
}

struct ExactTrust {
    hash: String,
    allowed: bool,
}
impl ExecutableTrust for ExactTrust {
    fn is_trusted(
        &self,
        _game: &QualifiedGameId,
        path: &str,
        hash: &str,
    ) -> Result<bool, FusionError> {
        Ok(self.allowed && path == "app/program.exe" && hash == self.hash)
    }
}

#[tokio::test]
async fn native_descriptor_uses_existing_supervisor_exact_approval_and_latest_rollback_state() {
    let fixture = CreatorHarness::synthetic().await;
    let input = fixture.library.join("native-input");
    fs::create_dir_all(&input).unwrap();
    let executable = std::env::current_exe().unwrap();
    fs::copy(&executable, input.join("program.exe")).unwrap();
    let activation = std::path::PathBuf::from(format!("{}.manifest", executable.display()));
    if activation.is_file() {
        fs::copy(activation, input.join("program.exe.manifest")).unwrap();
    }
    let object = CasStore::open(&fixture.library)
        .unwrap()
        .import_directory(&input)
        .unwrap();
    let hash = object
        .files
        .iter()
        .find(|file| file.path == "program.exe")
        .unwrap()
        .blake3
        .clone();
    let mut document = super::integration_tests::manifest();
    document.layers.get_mut("runtime").unwrap().critical_files[0].blake3 = hash.clone();
    let mut wire = serde_json::to_value(&document).unwrap();
    wire["creator"]["program_descriptor"]["entrypoint"]["arguments"] = serde_json::json!([
        {"kind":"literal","value":"--exact"}, {"kind":"literal","value":HELPER}, {"kind":"literal","value":"--nocapture"}
    ]);
    wire["creator"]["program_descriptor"]["entrypoint"]["environment_paths"] = serde_json::json!([
        {"name":"FUSION_DECLARATIVE_DATA","path":"userdata"}
    ]);
    document = serde_json::from_value(wire).unwrap();
    let program = super::runtime::descriptor(&document).unwrap();
    let approval = super::runtime::EnvironmentApproval::for_descriptor(
        program,
        BTreeSet::from(["FUSION_DECLARATIVE_DATA".into()]),
    )
    .unwrap();
    let game = QualifiedGameId {
        source_id: document.source_id.clone(),
        game_id: document.id.clone(),
    };
    let assembler = WorkspaceAssembler::open(&fixture.library, &game)
        .unwrap()
        .with_program_environment_approval(approval);
    let inputs = BTreeMap::from([(
        "runtime".into(),
        LayerInput::Cas {
            digest: object.digest,
            runtime_lock: None,
        },
    )]);
    let roots = PersistenceRoots {
        appdata: fixture.library.clone(),
        local_appdata: fixture.library.clone(),
    };
    let context = TemplateContext {
        workspace_dir: fixture.library.clone(),
        appdata_dir: fixture.library.clone(),
        local_appdata_dir: fixture.library.clone(),
        display_width: 1280,
        display_height: 720,
        refresh_rate: 60,
        enabled_addons: BTreeSet::new(),
    };
    let options = PlanOptions {
        build_profile_id: Some("standard".into()),
        controls_profile_id: Some("native".into()),
        ..PlanOptions::default()
    };
    let plan =
        PlanBuilder::build(&document, &options, &CapabilityRegistry::current_client()).unwrap();
    let first = assembler
        .assemble(
            &document,
            &plan,
            &inputs,
            &context,
            &roots,
            &WorkspaceBuildOptions::default(),
        )
        .unwrap();
    let original = first.snapshot.workspace_root.clone();
    let denied = ProcessSupervisor::launch(
        first,
        Arc::new(ExactTrust {
            hash: hash.clone(),
            allowed: false,
        }),
        None,
        SupervisorOptions::default(),
    )
    .await
    .unwrap()
    .wait()
    .await;
    assert!(
        matches!(denied, Err(ref error) if error.code == ErrorCode::UntrustedExecutable),
        "{denied:?}"
    );
    let outcome = ProcessSupervisor::launch(
        assembler.active().unwrap().unwrap(),
        Arc::new(ExactTrust {
            hash: hash.clone(),
            allowed: true,
        }),
        None,
        SupervisorOptions::default(),
    )
    .await
    .unwrap()
    .wait()
    .await
    .unwrap();
    assert_eq!(outcome.primary_exit_code, 0);
    assert_eq!(fs::read(original.join("userdata/save.dat")).unwrap(), b"1");
    document.version = "2".into();
    let updated = assembler
        .assemble(
            &document,
            &PlanBuilder::build(&document, &options, &CapabilityRegistry::current_client())
                .unwrap(),
            &inputs,
            &context,
            &roots,
            &WorkspaceBuildOptions::default(),
        )
        .unwrap();
    let outcome = ProcessSupervisor::launch(
        updated,
        Arc::new(ExactTrust {
            hash,
            allowed: true,
        }),
        None,
        SupervisorOptions::default(),
    )
    .await
    .unwrap()
    .wait()
    .await
    .unwrap();
    assert_eq!(outcome.primary_exit_code, 0);
    let rollback = assembler.rollback().unwrap();
    assert_eq!(rollback.snapshot.workspace_root, original);
    assert_eq!(
        fs::read(rollback.snapshot.workspace_root.join("userdata/save.dat")).unwrap(),
        b"2"
    );
}

#[tokio::test]
async fn legacy_h_snapshot_without_environment_approval_field_still_deserializes_and_verifies() {
    let fixture = CreatorHarness::synthetic().await;
    fixture
        .service
        .game_install(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    let snapshot = fixture.snapshot();
    let mut json = serde_json::to_value(&snapshot).unwrap();
    assert!(json.get("programEnvironmentApproval").is_none());
    json.as_object_mut()
        .unwrap()
        .remove("programEnvironmentApproval");
    let restored: WorkspaceSnapshot = serde_json::from_value(json).unwrap();
    assert!(restored.program_environment_approval.is_none());
    assert_eq!(restored.canonical_blake3, snapshot.canonical_blake3);
    crate::fusion_source::integrity::PreLaunchIntegrityGuard::verify_integrity(&restored).unwrap();
}
