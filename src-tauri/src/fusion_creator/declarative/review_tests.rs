//! Reproductions of independent-review findings, using the existing H environment.
use super::integration_tests::manifest;
use crate::fusion_source::cas::CasStore;
use crate::fusion_source::config_templates::TemplateContext;
use crate::fusion_source::fs_safety;
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::persistence::PersistenceRoots;
use crate::fusion_source::test_support::CreatorHarness;
use crate::fusion_source::workspace::{
    AssembledWorkspace, WorkspaceAssembler, WorkspaceBuildOptions,
};
use crate::fusion_source::{CapabilityRegistry, PlanBuilder, PlanOptions, QualifiedGameId};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

async fn assemble(
    change: impl FnOnce(&mut Value),
) -> (CreatorHarness, WorkspaceAssembler, AssembledWorkspace) {
    let fixture = CreatorHarness::synthetic().await;
    let input = fixture.library.join("review-input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("program.exe"), b"program bytes").unwrap();
    let object = CasStore::open(&fixture.library)
        .unwrap()
        .import_directory(&input)
        .unwrap();
    let mut wire = serde_json::to_value(manifest()).unwrap();
    change(&mut wire["creator"]["program_descriptor"]);
    let document = serde_json::from_value(wire).unwrap();
    let game = QualifiedGameId {
        source_id: "generic-source".into(),
        game_id: "generic".into(),
    };
    let assembler = WorkspaceAssembler::open(&fixture.library, &game).unwrap();
    let options = PlanOptions {
        build_profile_id: Some("standard".into()),
        controls_profile_id: Some("native".into()),
        ..Default::default()
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
    let inputs = BTreeMap::from([(
        "runtime".into(),
        LayerInput::Cas {
            digest: object.digest,
            runtime_lock: None,
        },
    )]);
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
    (fixture, assembler, lease)
}

struct TrustExact;
impl crate::fusion_source::integrity::ExecutableTrust for TrustExact {
    fn is_trusted(
        &self,
        _: &QualifiedGameId,
        path: &str,
        digest: &str,
    ) -> Result<bool, crate::fusion_source::FusionError> {
        Ok(path == "app/program.exe" && digest == blake3::hash(b"program bytes").to_hex().as_str())
    }
}

#[tokio::test]
async fn undeclared_loadables_in_application_and_private_save_roots_fail_integrity_and_launch() {
    use crate::fusion_source::integrity::PreLaunchIntegrityGuard;
    use crate::fusion_source::supervisor::{ProcessSupervisor, SupervisorOptions};
    for relative in ["app/version.dll", "userdata/saves/plugin.dll"] {
        let (_fixture, _assembler, lease) = assemble(|descriptor| {
            descriptor["persistence"][0]["path"] = "userdata/saves".into();
            descriptor["persistence"][0]["kind"] = "directory".into();
        })
        .await;
        PreLaunchIntegrityGuard::verify(&lease.snapshot, &TrustExact).unwrap();
        let path = lease.snapshot.workspace_root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"undeclared loadable").unwrap();
        let checked = PreLaunchIntegrityGuard::verify(&lease.snapshot, &TrustExact);
        let outcome = ProcessSupervisor::launch(
            lease,
            std::sync::Arc::new(TrustExact),
            None,
            SupervisorOptions::default(),
        )
        .await
        .unwrap()
        .wait()
        .await;
        assert!(
            matches!(checked, Err(ref error) if error.code == crate::fusion_source::ErrorCode::IntegrityViolation),
            "{checked:?}"
        );
        assert!(
            matches!(outcome, Err(ref error) if error.code == crate::fusion_source::ErrorCode::IntegrityViolation),
            "{outcome:?}"
        );
    }
}

#[cfg(windows)]
#[tokio::test]
async fn argument_only_junction_is_rejected_before_launch() {
    use crate::fusion_source::integrity::PreLaunchIntegrityGuard;
    let (fixture, _assembler, lease) = assemble(|descriptor| {
        descriptor["directories"]
            .as_array_mut()
            .unwrap()
            .push(json!("optional/data"));
        descriptor["entrypoint"]["arguments"] =
            json!([{"kind":"workspace_path", "path":"optional/data", "prefix":"--data="}]);
    })
    .await;
    let root = &lease.snapshot.workspace_root;
    PreLaunchIntegrityGuard::verify(&lease.snapshot, &TrustExact).unwrap();
    fs::remove_dir(root.join("optional/data")).unwrap();
    fs::remove_dir(root.join("optional")).unwrap();
    let outside = fixture.library.join("outside");
    fs::create_dir_all(outside.join("data")).unwrap();
    let linked = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(root.join("optional").to_string_lossy().replace('/', "\\"))
        .arg(outside.to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(linked.status.success(), "{linked:?}");
    let checked = PreLaunchIntegrityGuard::verify(&lease.snapshot, &TrustExact);
    fs::remove_dir(root.join("optional")).unwrap();
    assert!(checked.is_err(), "argument path escaped via a junction");
}

#[tokio::test]
async fn missing_argument_path_is_fail_closed_at_launch() {
    use crate::fusion_source::integrity::PreLaunchIntegrityGuard;
    let (_fixture, _assembler, lease) = assemble(|descriptor| {
        descriptor["directories"]
            .as_array_mut()
            .unwrap()
            .push(json!("optional/data"));
        descriptor["entrypoint"]["arguments"] =
            json!([{"kind":"workspace_path", "path":"optional/data", "prefix":""}]);
    })
    .await;
    fs::remove_dir(lease.snapshot.workspace_root.join("optional/data")).unwrap();
    assert!(PreLaunchIntegrityGuard::verify(&lease.snapshot, &TrustExact).is_err());
}

fn two_configs(descriptor: &mut Value) {
    for id in ["first", "second"] {
        descriptor["persistence"].as_array_mut().unwrap().push(json!({
            "path":format!("userdata/{id}.ini"), "kind":"file", "policy":"preserve_latest", "export":"never"
        }));
    }
    descriptor["configs"] = json!(["first", "second"].map(|id| json!({
        "id":id, "path":format!("userdata/{id}.ini"), "format":"ini", "seed":{"kind":"empty_ini"}, "preserve_unmanaged":true,
        "keys":[{"id":"flag", "address":{"kind":"ini","section":"Video","key":"flag"}, "scalar_type":"boolean", "allowed_values":[true,false]}]
    })));
    descriptor["build_profiles"][0]["settings"] =
        json!(["first", "second"].map(|id| json!({"config_id":id,"key_id":"flag","value":true})));
}

fn fingerprint(root: &std::path::Path) -> BTreeMap<String, (String, fs_safety::FileIdentity)> {
    fs_safety::list_plain_files(root)
        .unwrap()
        .into_iter()
        .map(|(name, path)| {
            let hash = blake3::hash(&fs::read(&path).unwrap()).to_hex().to_string();
            (name, (hash, fs_safety::file_identity(&path).unwrap()))
        })
        .collect()
}

#[tokio::test]
async fn inspecting_active_program_never_rewrites_configs_saves_or_metadata() {
    let (fixture, assembler, lease) = assemble(two_configs).await;
    let root = lease.snapshot.workspace_root.clone();
    fs::write(root.join("userdata/save.dat"), b"latest save").unwrap();
    lease.reconcile_saves().unwrap();
    fs::write(
        root.join("userdata/first.ini"),
        b"[Video]\nflag=false\n; private edit\n",
    )
    .unwrap();
    drop(lease);
    let before = fingerprint(&fixture.library);
    assembler
        .inspect_active(|lease, _| {
            assert!(lease.is_some());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        fingerprint(&fixture.library),
        before,
        "inspection mutated the installation"
    );
    let active = assembler.active().unwrap().unwrap();
    drop(active);
    assert_eq!(
        fingerprint(&fixture.library),
        before,
        "reacquire mutated the installation"
    );
}

#[tokio::test]
async fn failed_active_inspection_preserves_earlier_config_and_all_private_identities() {
    let (fixture, assembler, lease) = assemble(two_configs).await;
    let root = lease.snapshot.workspace_root.clone();
    fs::write(root.join("userdata/save.dat"), b"latest save").unwrap();
    lease.reconcile_saves().unwrap();
    fs::write(
        root.join("userdata/first.ini"),
        b"[Video]\nflag=false\n; private edit\n",
    )
    .unwrap();
    fs::write(
        root.join("userdata/second.ini"),
        b"[Video]\nflag=true\nflag=false\n",
    )
    .unwrap();
    fs_safety::unlink_file(&root.join("app/program.exe")).unwrap();
    drop(lease);
    let before = fingerprint(&fixture.library);
    assert!(assembler.inspect_active(|_, _| Ok(())).is_err());
    assert_eq!(
        fingerprint(&fixture.library),
        before,
        "failed inspection changed earlier state"
    );
}
