use super::integration_tests::manifest;
use crate::fusion_source::cas::CasStore;
use crate::fusion_source::config_templates::TemplateContext;
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::persistence::PersistenceRoots;
use crate::fusion_source::test_support::CreatorHarness;
use crate::fusion_source::workspace::{WorkspaceAssembler, WorkspaceBuildOptions};
use crate::fusion_source::{CapabilityRegistry, PlanBuilder, PlanOptions, QualifiedGameId};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[path = "external_acquisition_tests.rs"]
mod acquisition;

#[test]
fn external_program_plan_uses_pinned_descriptor_acquisition() {
    let mut value = serde_json::to_value(manifest()).unwrap();
    value["creator"]["program_descriptor"]["components"][0]["origin"] = serde_json::json!({
        "kind":"external", "url":"https://example.test/pinned.zip",
        "transport_blake3":"a".repeat(64), "inventory_blake3":"b".repeat(64)
    });
    let manifest = serde_json::from_value(value).unwrap();
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
    assert!(plan.steps.iter().any(|step| matches!(step,
        crate::fusion_source::plan::PlanStep::Fetch(
            crate::fusion_source::plan::FetchStep::AcquireLayer {
                layer_id, source: crate::fusion_source::schema::LayerSource::Remote { mirrors, archive },
                transport_digest: Some(digest),
            }) if layer_id == "runtime" && mirrors == &["https://example.test/pinned.zip"]
                && *archive == crate::fusion_source::schema::ArchiveFormat::Zip
                && digest.value == "a".repeat(64)
                && digest.algorithm == crate::fusion_source::schema::DigestAlgorithm::Blake3
    )));
}

#[tokio::test]
async fn integrity_and_approval_follow_resolved_provider_not_layer_order() {
    let fixture = CreatorHarness::synthetic().await;
    let cas = CasStore::open(&fixture.library).unwrap();
    let mut inputs = BTreeMap::new();
    let mut document = manifest();
    for (id, bytes) in [
        ("runtime", b"program bytes".as_slice()),
        ("replacement", b"different program".as_slice()),
    ] {
        let directory = fixture.library.join(format!("input-{id}"));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("program.exe"), bytes).unwrap();
        let object = cas.import_directory(&directory).unwrap();
        inputs.insert(
            id.into(),
            LayerInput::Cas {
                digest: object.digest,
                runtime_lock: None,
            },
        );
        if id == "replacement" {
            let mut layer = document.layers["runtime"].clone();
            layer.id = id.into();
            layer.critical_files[0].blake3 = blake3::hash(bytes).to_hex().to_string();
            document.layers.insert(id.into(), layer);
            document.layer_order.push(id.into());
        }
    }
    let mut wire = serde_json::to_value(&document).unwrap();
    let creator = &mut wire["creator"]["program_descriptor"];
    let mut component = creator["components"][0].clone();
    component["id"] = "replacement".into();
    component["layer_id"] = "replacement".into();
    creator["components"]
        .as_array_mut()
        .unwrap()
        .push(component);
    creator["build_profiles"][0]["layer_ids"] = serde_json::json!(["runtime", "replacement"]);
    creator["projection_rules"] =
        serde_json::json!([{"path":"app/program.exe","winning_layer":"runtime"}]);
    document = serde_json::from_value(wire).unwrap();
    let game = QualifiedGameId {
        source_id: document.source_id.clone(),
        game_id: document.id.clone(),
    };
    let assembler = WorkspaceAssembler::open(&fixture.library, &game).unwrap();
    let options = PlanOptions {
        build_profile_id: Some("standard".into()),
        controls_profile_id: Some("native".into()),
        ..PlanOptions::default()
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
    let roots = PersistenceRoots {
        appdata: fixture.library.clone(),
        local_appdata: fixture.library.clone(),
    };
    let lease = assembler
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
    assert_eq!(
        fs::read(lease.snapshot.workspace_root.join("app/program.exe")).unwrap(),
        b"program bytes"
    );
    let expected = crate::fusion_source::service::critical_inventory(&lease.snapshot);
    assert_eq!(expected.len(), 1);
    assert_eq!(
        expected[0].1.blake3,
        blake3::hash(b"program bytes").to_hex().to_string()
    );
}
