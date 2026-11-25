use crate::fusion_source::{CapabilityRegistry, PlanBuilder, PlanOptions, SchemaUpcaster};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn candidate_update_and_rollback_keep_latest_file_state_without_sharing_private_inodes() {
    use crate::fusion_source::{
        cas::CasStore,
        config_templates::TemplateContext,
        layers::LayerInput,
        persistence::PersistenceRoots,
        workspace::{WorkspaceAssembler, WorkspaceBuildOptions},
    };
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("library");
    let input = temp.path().join("input");
    let original = temp.path().join("original");
    std::fs::create_dir(&input).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::write(input.join("program.exe"), b"program bytes").unwrap();
    let object = CasStore::open(&library)
        .unwrap()
        .import_directory(&input)
        .unwrap();
    let mut document = manifest();
    let game = crate::fusion_source::QualifiedGameId {
        source_id: document.source_id.clone(),
        game_id: document.id.clone(),
    };
    let assembler = WorkspaceAssembler::open(&library, &game).unwrap();
    let inputs = BTreeMap::from([(
        "runtime".into(),
        LayerInput::Cas {
            digest: object.digest,
            runtime_lock: None,
        },
    )]);
    let roots = PersistenceRoots {
        appdata: original.clone(),
        local_appdata: original.clone(),
    };
    let context = TemplateContext {
        workspace_dir: library.clone(),
        appdata_dir: original.clone(),
        local_appdata_dir: original,
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
    let first = assembler
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
    let first_root = first.snapshot.workspace_root.clone();
    assert!(!first_root.join("userdata/save.dat").exists());
    std::fs::write(first_root.join("userdata/save.dat"), b"latest first").unwrap();
    first.reconcile_saves().unwrap();
    drop(first);
    document.version = "2".into();
    let second = assembler
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
    let second_root = second.snapshot.workspace_root.clone();
    assert_eq!(
        std::fs::read(second_root.join("userdata/save.dat")).unwrap(),
        b"latest first"
    );
    std::fs::write(second_root.join("userdata/save.dat"), b"latest second").unwrap();
    second.reconcile_saves().unwrap();
    drop(second);
    let rollback = assembler.rollback().unwrap();
    assert_eq!(rollback.snapshot.workspace_root, first_root);
    assert_eq!(
        std::fs::read(rollback.snapshot.workspace_root.join("userdata/save.dat")).unwrap(),
        b"latest second"
    );
    assert_ne!(
        crate::fusion_source::fs_safety::file_identity(&first_root.join("userdata/save.dat"))
            .unwrap(),
        crate::fusion_source::fs_safety::file_identity(&second_root.join("userdata/save.dat"))
            .unwrap()
    );
}

pub(super) fn manifest() -> crate::fusion_source::CanonicalGameManifest {
    let descriptor = json!({
        "document_type":"program_descriptor","schema_version":"1.0",
        "identity":{"id":"generic","title":"Generic","version":"1","os":"windows","architecture":"x86_64"},
        "executor":{"id":"fusion.declarative","version":"1.0"},
        "components":[{"id":"runtime","layer_id":"runtime","origin":{"kind":"managed","artifact_ref":"runtime","redistribution_allowed":false},"source_root":"","mount_path":"app"}],
        "required_files":[{"path":"app/program.exe","role":"main_executable"}],
        "projection_rules":[],"directories":["userdata"],
        "entrypoint":{"executable":"app/program.exe","working_directory":"app","arguments":[],"environment_paths":[]},
        "configs":[],"build_profiles":[{"id":"standard","title":"Standard","layer_ids":["runtime"],"controls_profile_ids":["native"],"settings":[]}],
        "controls_profiles":[{"id":"native","title":"Application defaults","kind":"application_defaults"}],
        "persistence":[{"path":"userdata/save.dat","kind":"file","policy":"preserve_latest","export":"never"}]
    });
    SchemaUpcaster::upcast_game_manifest("generic-source",&json!({
        "schema_version":"1.1","id":"generic","title":"Generic","version":"1",
        "creator":{"program_descriptor":descriptor},
        "layers":[{"id":"runtime","kind":"runtime","mount_path":"app","source":{"kind":"user_provided"},"critical_files":[{"path":"program.exe","role":"main_executable","blake3":blake3::hash(b"program bytes").to_hex().to_string()}]}]
    }).to_string(),&BTreeMap::new()).unwrap()
}

#[test]
fn explicit_program_pair_compiles_into_the_existing_plan_without_legacy_fallback() {
    let manifest = manifest();
    let plan = PlanBuilder::build(
        &manifest,
        &PlanOptions {
            build_profile_id: Some("standard".into()),
            controls_profile_id: Some("native".into()),
            ..PlanOptions::default()
        },
        &CapabilityRegistry::current_client(),
    )
    .unwrap();
    assert_eq!(plan.selected_layers, vec!["runtime"]);
    assert_eq!(plan.build_profile_id.as_deref(), Some("standard"));
    assert!(plan.steps.iter().any(|step| matches!(
        step,
        crate::fusion_source::plan::PlanStep::Exec(
            crate::fusion_source::plan::ExecStep::StartProcess { .. }
        )
    )));
    assert!(PlanBuilder::build(
        &manifest,
        &PlanOptions {
            build_profile_id: Some("standard".into()),
            controls_profile_id: Some("native".into()),
            enabled_addons: Some(BTreeSet::new()),
            ..PlanOptions::default()
        },
        &CapabilityRegistry::current_client()
    )
    .is_err());
}
