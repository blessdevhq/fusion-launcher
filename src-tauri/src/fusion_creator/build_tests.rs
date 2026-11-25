use super::*;
#[path = "build_input_safety_tests.rs"]
mod input_safety;
use crate::fusion_creator::{
    project_service, project_storage, CreatorJobKind, CreatorJobStartRequest, CreatorJobStatus,
    CreatorPrivateBindings,
};
use crate::fusion_source::{signatures, ErrorCode};
use crate::storage::RepositoryStore;
use std::{
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

struct Fixture {
    _dir: tempfile::TempDir,
    runner: JobRunner,
    request: CreatorJobStartRequest,
    input: PathBuf,
    key: PathBuf,
}

impl Fixture {
    fn new(origin: serde_json::Value, bind_key: bool) -> Self {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("private-owned-input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("app.exe"), b"synthetic owned executable").unwrap();
        let key = dir.path().join("private-signing-key.pem");
        signatures::keygen(&key).unwrap();
        let mut store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
        let project = json!({
            "schema_version":"1.1","project_id":"build-fixture","revision":1,"title":"Build fixture",
            "source_id":"signed-fixture","game_id":"build-demo","game_version":"1",
            "program_descriptor":{
                "document_type":"program_descriptor","schema_version":"1.0",
                "identity":{"id":"demo","title":"Demo","version":"1","os":"windows","architecture":"x86_64"},
                "executor":{"id":"fusion.declarative","version":"1.0"},
                "components":[{"id":"program","layer_id":"runtime","origin":origin,"source_root":"","mount_path":"app"}],
                "required_files":[{"path":"app/app.exe","role":"main_executable"}],
                "projection_rules":[],"directories":[],
                "entrypoint":{"executable":"app/app.exe","working_directory":"app","arguments":[],"environment_paths":[]},
                "configs":[],"persistence":[],
                "build_profiles":[{"id":"normal","title":"Normal","layer_ids":["runtime"],"controls_profile_ids":["native"],"settings":[]}],
                "controls_profiles":[{"id":"native","title":"Native","kind":"application_defaults"}]
            }
        });
        project_storage::save_project_bytes(
            &mut store,
            &serde_json::to_vec(&project).unwrap(),
            None,
        )
        .unwrap();
        project_storage::save_private_bindings(
            &store,
            1,
            None,
            &CreatorPrivateBindings {
                project_id: "build-fixture".into(),
                component_paths: [("program".into(), input.to_str().unwrap().into())].into(),
                signing_key_ref: bind_key.then(|| key.to_str().unwrap().into()),
            },
        )
        .unwrap();
        let runner = production_runner(
            Arc::new(Mutex::new(store)),
            &dir.path().join("creator"),
            Arc::new(|_| {}),
        )
        .unwrap();
        Self {
            _dir: dir,
            runner,
            input,
            key,
            request: CreatorJobStartRequest {
                project_id: "build-fixture".into(),
                project_revision: 1,
                build_profile_ids: vec!["normal".into()],
                controls_profile_ids: vec!["native".into()],
                release_id: None,
                target_directory: None,
                source_url: None,
            },
        }
    }
    fn releases(&self) -> Vec<crate::fusion_creator::CreatorRelease> {
        project_service::release_list(
            &self.runner.store().lock().unwrap(),
            &self.request.project_id,
        )
        .unwrap()
    }
    fn start(&self) -> Result<crate::fusion_creator::CreatorJobReceipt, FusionError> {
        self.runner
            .start(CreatorJobKind::Build, self.request.clone())
    }
    fn start_verify(
        &self,
        request: CreatorJobStartRequest,
    ) -> crate::fusion_creator::CreatorJobReceipt {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.runner.start(CreatorJobKind::Verify, request.clone()) {
                Ok(receipt) => return receipt,
                // Terminal evidence becomes visible just before the prior worker
                // unwinds and releases its workspace lease.
                Err(error)
                    if error.code == ErrorCode::OperationInProgress
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("Verify start failed: {error:?}"),
            }
        }
    }
    fn verify(&self, release_id: &str) -> crate::fusion_creator::CreatorJob {
        let mut request = self.request.clone();
        request.release_id = Some(release_id.into());
        let receipt = self.start_verify(request);
        self.terminal(&receipt.job_id)
    }
    fn terminal(&self, id: &str) -> crate::fusion_creator::CreatorJob {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let job = self.runner.get(id).unwrap();
            if matches!(
                job.progress.status,
                CreatorJobStatus::Completed
                    | CreatorJobStatus::Failed
                    | CreatorJobStatus::Cancelled
            ) {
                return job;
            }
            assert!(Instant::now() < deadline, "Build worker timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn managed(allowed: bool) -> serde_json::Value {
    serde_json::json!({"kind":"managed","artifact_ref":"owned-fixture","redistribution_allowed":allowed})
}

#[tokio::test]
async fn production_build_returns_receipt_before_input_audit_failure() {
    let fixture = Fixture::new(managed(true), true);
    fs::remove_dir_all(&fixture.input).unwrap();
    fs::write(&fixture.input, b"not a directory").unwrap();
    let receipt = fixture.start().expect("input audit must run after queuing");
    let job = fixture.terminal(&receipt.job_id);
    assert_eq!(job.progress.status, CreatorJobStatus::Failed);
    assert_eq!(
        job.progress.error.unwrap().details["componentId"],
        "program"
    );
    assert!(job.checkpoints.is_empty());
    assert!(fixture.releases().is_empty());
}

#[tokio::test]
async fn production_build_commits_audited_managed_bytes() {
    let fixture = Fixture::new(managed(true), true);
    let receipt = fixture.start().unwrap();
    let job = fixture.terminal(&receipt.job_id);
    assert_eq!(
        job.progress.status,
        CreatorJobStatus::Completed,
        "{:?}",
        job.progress.error
    );
    let releases = fixture.releases();
    assert_eq!(releases.len(), 1);
    let release = &releases[0];
    assert_eq!(
        job.progress.release_id.as_deref(),
        Some(release.release_id.as_str())
    );
    fs::remove_dir_all(&fixture.input).unwrap();
    fs::remove_file(&fixture.key).unwrap();
    let content = fixture
        .runner
        .read_release_content(
            &release.project_id,
            release.project_revision,
            &release.release_id,
        )
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(content.manifest()).unwrap();
    assert_eq!(manifest["schema_version"], "1.1");
    assert_eq!(
        manifest["creator"]["program_descriptor"]["identity"]["id"],
        "demo"
    );
    assert_eq!(release.artifact_hashes.len(), 3);
    let mut index = String::new();
    use std::io::Read as _;
    content
        .open_artifact("source-index.json")
        .unwrap()
        .read_to_string(&mut index)
        .unwrap();
    let signed = signatures::parse_wire_index(&index).unwrap();
    assert!(signatures::verify_wire_index(&signed, None, None).unwrap());
    let parsed: crate::fusion_source::schema::SourceIndex = serde_json::from_str(&index).unwrap();
    assert_eq!(parsed.games[0].manifest_blake3, release.manifest_blake3);
    assert!(content.open_artifact("artifact-inventory.json").is_ok());
    let zip_id = release
        .artifact_hashes
        .keys()
        .find(|id| id.ends_with(".zip"))
        .unwrap();
    assert!(content.open_artifact(zip_id).is_ok());
}

#[tokio::test]
async fn production_build_fails_closed_without_redistribution_or_signing_binding() {
    for (allowed, bind_key, code) in [
        (false, true, ErrorCode::ActionUnavailable),
        (true, false, ErrorCode::InvalidSourceSignature),
    ] {
        let fixture = Fixture::new(managed(allowed), bind_key);
        let error = fixture.start().expect_err("unsupported Build succeeded");
        assert_eq!(error.code, code);
        assert!(fixture.releases().is_empty());
    }
}

#[tokio::test]
async fn production_verify_installs_the_pinned_release_without_author_inputs() {
    use crate::fusion_creator::{CreatorReportKind, CreatorReportStatus};
    let fixture = Fixture::new(managed(true), true);
    let built = fixture.terminal(&fixture.start().unwrap().job_id);
    assert_eq!(
        built.progress.status,
        CreatorJobStatus::Completed,
        "{:?}",
        built.progress.error
    );
    let release = fixture.releases().pop().unwrap();
    fs::remove_dir_all(&fixture.input).unwrap();
    fs::remove_file(&fixture.key).unwrap();
    let job = fixture.verify(&release.release_id);
    assert_eq!(
        job.progress.status,
        CreatorJobStatus::Completed,
        "{:?}",
        job.progress.error
    );
    assert_eq!(job.report_ids.len(), 2);
    let store = fixture.runner.store();
    let store = store.lock().unwrap();
    for (id, kind) in job.report_ids.iter().zip([
        CreatorReportKind::StructureIntegrity,
        CreatorReportKind::CleanInstall,
    ]) {
        let report = project_service::report_get(&store, id).unwrap();
        assert_eq!(report.kind, kind);
        assert_eq!(report.status, CreatorReportStatus::Passed);
        assert_eq!(report.release_id, release.release_id);
        assert_eq!(report.release_blake3, release.manifest_blake3);
        assert_eq!(
            (
                report.build_profile_id.as_str(),
                report.controls_profile_id.as_str()
            ),
            ("normal", "native")
        );
        assert!(report.diagnostics.is_empty());
    }
    drop(store);
    assert_eq!(fixture.releases()[0].report_ids.len(), 2);
}

#[tokio::test]
async fn production_verify_rejects_corrupt_saved_bytes_without_publishing_reports() {
    let fixture = Fixture::new(managed(true), true);
    let built = fixture.terminal(&fixture.start().unwrap().job_id);
    assert_eq!(built.progress.status, CreatorJobStatus::Completed);
    let release = fixture.releases().pop().unwrap();
    flip_cas_zip_byte(&fixture._dir.path().join("creator"), &release);
    let job = fixture.verify(&release.release_id);
    assert_eq!(job.progress.status, CreatorJobStatus::Failed);
    assert_eq!(
        job.progress.error.unwrap().code,
        ErrorCode::IntegrityViolation
    );
    assert!(job.report_ids.is_empty());
    assert!(job.checkpoints.is_empty());
    assert!(fixture.releases()[0].report_ids.is_empty());
}

fn flip_cas_zip_byte(creator_root: &Path, release: &crate::fusion_creator::CreatorRelease) {
    let (artifact_id, hash) = release
        .artifact_hashes
        .iter()
        .find(|(id, _)| id.ends_with(".zip"))
        .unwrap();
    let objects = creator_root.join(".fusion-source/cas/objects");
    let artifact = fs::read_dir(&objects)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("data/artifacts").join(artifact_id))
        .find(|path| path.is_file())
        .expect("saved CAS artifact");
    let mut bytes = fs::read(&artifact).unwrap();
    assert_eq!(blake3::hash(&bytes).to_hex().as_str(), hash);
    bytes[0] ^= 1;
    let original = fs::metadata(&artifact).unwrap().permissions();
    let mut writable = original.clone();
    #[cfg(windows)]
    #[allow(clippy::permissions_set_readonly_false)]
    writable.set_readonly(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        writable.set_mode(writable.mode() | 0o200);
    }
    fs::set_permissions(&artifact, writable).unwrap();
    fs::write(&artifact, bytes).unwrap();
    fs::set_permissions(&artifact, original).unwrap();
}

fn multi_profile_descriptor() -> serde_json::Value {
    serde_json::json!({
        "document_type":"program_descriptor","schema_version":"1.0",
        "identity":{"id":"multi-demo","title":"Multi Demo","version":"1.0","os":"windows","architecture":"x86_64"},
        "executor":{"id":"fusion.declarative","version":"1.0"},
        "components":[
            {"id":"program","layer_id":"runtime","origin":managed(true),"source_root":"","mount_path":"app"},
            {"id":"addon","layer_id":"addon","origin":managed(true),"source_root":"","mount_path":"app/addons"}
        ],
        "required_files":[{"path":"app/app.exe","role":"main_executable"}],
        "projection_rules":[],
        "directories":["data"],
        "entrypoint":{"executable":"app/app.exe","working_directory":"app","arguments":[],"environment_paths":[]},
        "configs":[
            {
                "id":"ini","path":"data/config.ini","format":"ini","seed":{"kind":"empty_ini"},
                "preserve_unmanaged":true,
                "keys":[
                    {"id":"fullscreen","address":{"kind":"ini","section":"Video","key":"fullscreen"},"scalar_type":"boolean","allowed_values":[false,true]},
                    {"id":"scale","address":{"kind":"ini","section":"Video","key":"scale"},"scalar_type":"integer","allowed_values":[1,2]},
                    {"id":"mode","address":{"kind":"ini","section":"General","key":"mode"},"scalar_type":"string","allowed_values":["standard","non-default"]}
                ]
            },
            {
                "id":"xml","path":"data/config.xml","format":"xml_attributes","seed":{"kind":"xml_root","root":"settings","attributes":[{"name":"version","value":"1"}]},
                "preserve_unmanaged":true,
                "keys":[
                    {"id":"quality","address":{"kind":"xml_attribute","elements":["Render"],"attribute":"quality"},"scalar_type":"string","allowed_values":["low","high"]},
                    {"id":"subtitles","address":{"kind":"xml_attribute","elements":["Audio"],"attribute":"subtitles"},"scalar_type":"boolean","allowed_values":[false,true]}
                ]
            }
        ],
        "persistence":[],
        "build_profiles":[
            {
                "id":"default","title":"Default","layer_ids":["runtime"],"controls_profile_ids":["native"],
                "settings":[
                    {"config_id":"ini","key_id":"fullscreen","value":true},
                    {"config_id":"ini","key_id":"scale","value":1},
                    {"config_id":"ini","key_id":"mode","value":"standard"},
                    {"config_id":"xml","key_id":"quality","value":"low"},
                    {"config_id":"xml","key_id":"subtitles","value":false}
                ]
            },
            {
                "id":"custom","title":"Custom Non-Default","layer_ids":["runtime","addon"],"controls_profile_ids":["native"],
                "settings":[
                    {"config_id":"ini","key_id":"fullscreen","value":false},
                    {"config_id":"ini","key_id":"scale","value":2},
                    {"config_id":"ini","key_id":"mode","value":"non-default"},
                    {"config_id":"xml","key_id":"quality","value":"high"},
                    {"config_id":"xml","key_id":"subtitles","value":true}
                ]
            }
        ],
        "controls_profiles":[{"id":"native","title":"Native","kind":"application_defaults"}]
    })
}

#[tokio::test]
async fn production_verify_validates_non_default_configs_and_profile_layers() {
    use crate::fusion_creator::{CreatorReportKind, CreatorReportStatus};
    let dir = tempfile::tempdir().unwrap();
    let runtime_dir = dir.path().join("runtime-input");
    let addon_dir = dir.path().join("addon-input");
    fs::create_dir(&runtime_dir).unwrap();
    fs::create_dir(&addon_dir).unwrap();
    fs::write(runtime_dir.join("app.exe"), b"synthetic program exe").unwrap();
    fs::write(addon_dir.join("extra.dat"), b"synthetic addon payload").unwrap();
    let key = dir.path().join("private-signing-key.pem");
    signatures::keygen(&key).unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
    let project = serde_json::json!({
        "schema_version":"1.1","project_id":"multi-fixture","revision":1,"title":"Multi Fixture",
        "source_id":"signed-multi","game_id":"multi-demo","game_version":"1.0",
        "program_descriptor": multi_profile_descriptor()
    });
    project_storage::save_project_bytes(&mut store, &serde_json::to_vec(&project).unwrap(), None)
        .unwrap();
    project_storage::save_private_bindings(
        &store,
        1,
        None,
        &CreatorPrivateBindings {
            project_id: "multi-fixture".into(),
            component_paths: [
                ("program".into(), runtime_dir.to_str().unwrap().into()),
                ("addon".into(), addon_dir.to_str().unwrap().into()),
            ]
            .into(),
            signing_key_ref: Some(key.to_str().unwrap().into()),
        },
    )
    .unwrap();
    let runner = production_runner(
        Arc::new(Mutex::new(store)),
        &dir.path().join("creator"),
        Arc::new(|_| {}),
    )
    .unwrap();
    let helper = Fixture {
        _dir: dir,
        runner,
        input: runtime_dir.clone(),
        key: key.clone(),
        request: CreatorJobStartRequest {
            project_id: "multi-fixture".into(),
            project_revision: 1,
            build_profile_ids: vec!["custom".into()],
            controls_profile_ids: vec!["native".into()],
            release_id: None,
            target_directory: None,
            source_url: None,
        },
    };
    let built = helper.terminal(&helper.start().unwrap().job_id);
    assert_eq!(built.progress.status, CreatorJobStatus::Completed);
    let release = helper.releases().pop().unwrap();
    fs::remove_dir_all(&runtime_dir).unwrap();
    fs::remove_dir_all(&addon_dir).unwrap();
    fs::remove_file(&key).unwrap();

    // First profile pair ("custom", "native") passes, project remains NotTested until all pairs pass.
    let custom_job = helper.verify(&release.release_id);
    assert_eq!(
        custom_job.progress.status,
        CreatorJobStatus::Completed,
        "{:?}",
        custom_job.progress.error
    );
    assert_eq!(custom_job.report_ids.len(), 2);
    assert_eq!(
        project_service::list(&helper.runner.store().lock().unwrap()).unwrap()[0]
            .verification_status,
        CreatorReportStatus::NotTested
    );

    // Second profile pair ("default", "native") passes -> all declared pairs verified -> Passed.
    let default_receipt = loop {
        match helper.runner.start(
            CreatorJobKind::Verify,
            CreatorJobStartRequest {
                project_id: "multi-fixture".into(),
                project_revision: 1,
                build_profile_ids: vec!["default".into()],
                controls_profile_ids: vec!["native".into()],
                release_id: Some(release.release_id.clone()),
                target_directory: None,
                source_url: None,
            },
        ) {
            Ok(receipt) => break receipt,
            Err(error) if error.code == ErrorCode::OperationInProgress => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("Verify default profile failed: {error:?}"),
        }
    };
    let default_job = helper.terminal(&default_receipt.job_id);
    assert_eq!(
        default_job.progress.status,
        CreatorJobStatus::Completed,
        "{:?}",
        default_job.progress.error
    );
    let store = helper.runner.store();
    let store = store.lock().unwrap();
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::Passed
    );
    for (id, kind) in default_job.report_ids.iter().zip([
        CreatorReportKind::StructureIntegrity,
        CreatorReportKind::CleanInstall,
    ]) {
        let report = project_service::report_get(&store, id).unwrap();
        assert_eq!(report.kind, kind);
        assert_eq!(report.status, CreatorReportStatus::Passed);
        assert_eq!(report.build_profile_id, "default");
    }
    drop(store);
    assert_eq!(helper.releases()[0].report_ids.len(), 4);
}

#[tokio::test]
async fn production_verify_refuses_env_paths_with_descriptor_bound_failure_and_keeps_structure_pass(
) {
    use crate::fusion_creator::{CreatorReportKind, CreatorReportStatus};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("private-owned-input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("app.exe"), b"synthetic owned executable").unwrap();
    let key = dir.path().join("private-signing-key.pem");
    signatures::keygen(&key).unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
    let descriptor = serde_json::json!({
        "document_type":"program_descriptor","schema_version":"1.0",
        "identity":{"id":"env-demo","title":"Env Demo","version":"1","os":"windows","architecture":"x86_64"},
        "executor":{"id":"fusion.declarative","version":"1.0"},
        "components":[{"id":"program","layer_id":"runtime","origin":managed(true),"source_root":"","mount_path":"app"}],
        "required_files":[{"path":"app/app.exe","role":"main_executable"}],
        "projection_rules":[],"directories":["data"],
        "entrypoint":{"executable":"app/app.exe","working_directory":"app","arguments":[],"environment_paths":[{"name":"DEMO_DATA_DIR","path":"data"}]},
        "configs":[],"persistence":[],
        "build_profiles":[{"id":"normal","title":"Normal","layer_ids":["runtime"],"controls_profile_ids":["native"],"settings":[]}],
        "controls_profiles":[{"id":"native","title":"Native","kind":"application_defaults"}]
    });
    let expected_descriptor_digest =
        blake3::hash(&serde_json::to_vec(&serde_json::to_value(&descriptor).unwrap()).unwrap())
            .to_hex()
            .to_string();
    let project = serde_json::json!({
        "schema_version":"1.1","project_id":"env-fixture","revision":1,"title":"Env fixture",
        "source_id":"signed-env","game_id":"env-demo","game_version":"1",
        "program_descriptor": descriptor
    });
    project_storage::save_project_bytes(&mut store, &serde_json::to_vec(&project).unwrap(), None)
        .unwrap();
    project_storage::save_private_bindings(
        &store,
        1,
        None,
        &CreatorPrivateBindings {
            project_id: "env-fixture".into(),
            component_paths: [("program".into(), input.to_str().unwrap().into())].into(),
            signing_key_ref: Some(key.to_str().unwrap().into()),
        },
    )
    .unwrap();
    let runner = production_runner(
        Arc::new(Mutex::new(store)),
        &dir.path().join("creator"),
        Arc::new(|_| {}),
    )
    .unwrap();
    let fixture = Fixture {
        _dir: dir,
        runner,
        input,
        key,
        request: CreatorJobStartRequest {
            project_id: "env-fixture".into(),
            project_revision: 1,
            build_profile_ids: vec!["normal".into()],
            controls_profile_ids: vec!["native".into()],
            release_id: None,
            target_directory: None,
            source_url: None,
        },
    };
    let built = fixture.terminal(&fixture.start().unwrap().job_id);
    assert_eq!(built.progress.status, CreatorJobStatus::Completed);
    let release = fixture.releases().pop().unwrap();

    let job = fixture.verify(&release.release_id);
    assert_eq!(job.progress.status, CreatorJobStatus::Failed);
    let err = job.progress.error.unwrap();
    assert_eq!(err.code, ErrorCode::ActionUnavailable);
    assert_eq!(err.details["stage"], "clean_install");
    assert_eq!(err.details["reason"], "environment_approval_required");
    assert_eq!(
        err.details["variables"],
        serde_json::json!(["DEMO_DATA_DIR"])
    );
    assert_eq!(err.details["descriptorBlake3"], expected_descriptor_digest);
    assert_eq!(
        err.details["passedStages"],
        serde_json::json!(["structure_integrity"])
    );
    assert_eq!(job.report_ids.len(), 2);
    let store = fixture.runner.store();
    let store = store.lock().unwrap();
    let r0 = project_service::report_get(&store, &job.report_ids[0]).unwrap();
    let r1 = project_service::report_get(&store, &job.report_ids[1]).unwrap();
    assert_eq!(r0.kind, CreatorReportKind::StructureIntegrity);
    assert_eq!(r0.status, CreatorReportStatus::Passed);
    assert_eq!(r1.kind, CreatorReportKind::CleanInstall);
    assert_eq!(r1.status, CreatorReportStatus::Failed);
    assert_eq!(r1.diagnostics.len(), 1);
    assert_eq!(r1.diagnostics[0].code, ErrorCode::ActionUnavailable);
    assert_eq!(
        r1.diagnostics[0].details["reason"],
        "environment_approval_required"
    );
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::Failed
    );
    drop(store);
    assert_eq!(fixture.releases()[0].report_ids.len(), 2);
}

#[tokio::test]
async fn production_verify_cancel_during_clean_install_retains_structure_report_and_cleans_scratch()
{
    use crate::fusion_creator::{CreatorReportKind, CreatorReportStatus};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("private-owned-input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("app.exe"), b"synthetic owned executable").unwrap();
    // Deterministic incompressible payload so clean_install spends >100ms in digest/extract/import.
    let mut blob = vec![0u8; 8 * 1024 * 1024];
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    for byte in &mut blob {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        *byte = (state >> 56) as u8;
    }
    fs::write(input.join("payload.bin"), &blob).unwrap();
    let key = dir.path().join("private-signing-key.pem");
    signatures::keygen(&key).unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
    let project = serde_json::json!({
        "schema_version":"1.1","project_id":"build-fixture","revision":1,"title":"Build fixture",
        "source_id":"signed-fixture","game_id":"build-demo","game_version":"1",
        "program_descriptor":{
            "document_type":"program_descriptor","schema_version":"1.0",
            "identity":{"id":"demo","title":"Demo","version":"1","os":"windows","architecture":"x86_64"},
            "executor":{"id":"fusion.declarative","version":"1.0"},
            "components":[{"id":"program","layer_id":"runtime","origin":managed(true),"source_root":"","mount_path":"app"}],
            "required_files":[{"path":"app/app.exe","role":"main_executable"}],
            "projection_rules":[],"directories":[],
            "entrypoint":{"executable":"app/app.exe","working_directory":"app","arguments":[],"environment_paths":[]},
            "configs":[],"persistence":[],
            "build_profiles":[{"id":"normal","title":"Normal","layer_ids":["runtime"],"controls_profile_ids":["native"],"settings":[]}],
            "controls_profiles":[{"id":"native","title":"Native","kind":"application_defaults"}]
        }
    });
    project_storage::save_project_bytes(&mut store, &serde_json::to_vec(&project).unwrap(), None)
        .unwrap();
    project_storage::save_private_bindings(
        &store,
        1,
        None,
        &CreatorPrivateBindings {
            project_id: "build-fixture".into(),
            component_paths: [("program".into(), input.to_str().unwrap().into())].into(),
            signing_key_ref: Some(key.to_str().unwrap().into()),
        },
    )
    .unwrap();
    let (clean_tx, clean_rx) = std::sync::mpsc::sync_channel::<String>(1);
    let runner = production_runner(
        Arc::new(Mutex::new(store)),
        &dir.path().join("creator"),
        Arc::new(move |progress| {
            if progress.kind == CreatorJobKind::Verify
                && progress.phase == "clean_install"
                && progress.completed_units == 1
            {
                let _ = clean_tx.send(progress.job_id);
            }
        }),
    )
    .unwrap();
    let fixture = Fixture {
        _dir: dir,
        runner,
        input,
        key,
        request: CreatorJobStartRequest {
            project_id: "build-fixture".into(),
            project_revision: 1,
            build_profile_ids: vec!["normal".into()],
            controls_profile_ids: vec!["native".into()],
            release_id: None,
            target_directory: None,
            source_url: None,
        },
    };
    let built = fixture.terminal(&fixture.start().unwrap().job_id);
    assert_eq!(built.progress.status, CreatorJobStatus::Completed);
    let release = fixture.releases().pop().unwrap();
    let mut req = fixture.request.clone();
    req.release_id = Some(release.release_id.clone());
    let verify_receipt = fixture.start_verify(req);
    let signalled = clean_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(signalled, verify_receipt.job_id);
    let _ = fixture.runner.cancel(&verify_receipt.job_id).unwrap();
    let job = fixture.terminal(&verify_receipt.job_id);
    assert_eq!(job.progress.status, CreatorJobStatus::Cancelled);
    assert_eq!(job.report_ids.len(), 1);
    let store = fixture.runner.store();
    let store = store.lock().unwrap();
    let report = project_service::report_get(&store, &job.report_ids[0]).unwrap();
    assert_eq!(report.kind, CreatorReportKind::StructureIntegrity);
    assert_eq!(report.status, CreatorReportStatus::Passed);
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::NotTested
    );
    drop(store);
    assert_eq!(fixture.releases()[0].report_ids.len(), 1);
    let workspace = fixture
        ._dir
        .path()
        .join("creator/workspaces")
        .join(blake3::hash(b"build-fixture").to_hex().as_str());
    let leftover_scratch: Vec<_> = fs::read_dir(&workspace)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("verify"))
        .collect();
    assert!(
        leftover_scratch.is_empty(),
        "verify scratch dir remained after cancel: {leftover_scratch:?}"
    );
}

const VERIFY_CRASH_CHILD_ROOT: &str = "FUSION_CREATOR_VERIFY_CRASH_CHILD_ROOT";

#[tokio::test]
async fn production_verify_resumes_interrupted_job_and_reaudits_saved_bytes() {
    use crate::fusion_creator::{CreatorReportKind, CreatorReportStatus};
    if let Some(root) = std::env::var_os(VERIFY_CRASH_CHILD_ROOT) {
        let root = PathBuf::from(root);
        let input = root.join("private-owned-input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("app.exe"), b"synthetic owned executable").unwrap();
        let key = root.join("private-signing-key.pem");
        signatures::keygen(&key).unwrap();
        let mut store = RepositoryStore::open(&root.join("creator.sqlite")).unwrap();
        let project = serde_json::json!({
            "schema_version":"1.1","project_id":"build-fixture","revision":1,"title":"Build fixture",
            "source_id":"signed-fixture","game_id":"build-demo","game_version":"1",
            "program_descriptor":{
                "document_type":"program_descriptor","schema_version":"1.0",
                "identity":{"id":"demo","title":"Demo","version":"1","os":"windows","architecture":"x86_64"},
                "executor":{"id":"fusion.declarative","version":"1.0"},
                "components":[{"id":"program","layer_id":"runtime","origin":managed(true),"source_root":"","mount_path":"app"}],
                "required_files":[{"path":"app/app.exe","role":"main_executable"}],
                "projection_rules":[],"directories":[],
                "entrypoint":{"executable":"app/app.exe","working_directory":"app","arguments":[],"environment_paths":[]},
                "configs":[],"persistence":[],
                "build_profiles":[{"id":"normal","title":"Normal","layer_ids":["runtime"],"controls_profile_ids":["native"],"settings":[]}],
                "controls_profiles":[{"id":"native","title":"Native","kind":"application_defaults"}]
            }
        });
        project_storage::save_project_bytes(
            &mut store,
            &serde_json::to_vec(&project).unwrap(),
            None,
        )
        .unwrap();
        project_storage::save_private_bindings(
            &store,
            1,
            None,
            &CreatorPrivateBindings {
                project_id: "build-fixture".into(),
                component_paths: [("program".into(), input.to_str().unwrap().into())].into(),
                signing_key_ref: Some(key.to_str().unwrap().into()),
            },
        )
        .unwrap();
        let runner = production_runner(
            Arc::new(Mutex::new(store)),
            &root.join("creator"),
            Arc::new(|progress| {
                if progress.kind == CreatorJobKind::Verify && progress.checkpoint_id.is_some() {
                    std::process::exit(98);
                }
            }),
        )
        .unwrap();
        let req = CreatorJobStartRequest {
            project_id: "build-fixture".into(),
            project_revision: 1,
            build_profile_ids: vec!["normal".into()],
            controls_profile_ids: vec!["native".into()],
            release_id: None,
            target_directory: None,
            source_url: None,
        };
        let built_receipt = runner.start(CreatorJobKind::Build, req.clone()).unwrap();
        let built = loop {
            let job = runner.get(&built_receipt.job_id).unwrap();
            if job.progress.status == CreatorJobStatus::Completed {
                break job;
            }
            assert_ne!(job.progress.status, CreatorJobStatus::Failed);
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut verify_req = req;
        verify_req.release_id = built.progress.release_id;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match runner.start(CreatorJobKind::Verify, verify_req.clone()) {
                Ok(_) => break,
                Err(error)
                    if error.code == ErrorCode::OperationInProgress
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("Child verify start failed: {error:?}"),
            }
        }
        std::thread::sleep(Duration::from_secs(15));
        panic!("child should have exited right after StructureIntegrity checkpoint");
    }

    let spawn_interrupted_verify = || -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fusion_creator::bridge::build_tests::production_verify_resumes_interrupted_job_and_reaudits_saved_bytes",
                "--nocapture",
            ])
            .env(VERIFY_CRASH_CHILD_ROOT, dir.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(98));
        let store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
        let job_id: String = store
            .conn
            .query_row(
                "SELECT job_id FROM creator_jobs WHERE status='running'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (dir, job_id)
    };

    // 1. Happy path: interrupted after StructureIntegrity -> resume completes with exactly 2 reports.
    {
        let (dir, job_id) = spawn_interrupted_verify();
        let store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
        let runner = production_runner(
            Arc::new(Mutex::new(store)),
            &dir.path().join("creator"),
            Arc::new(|_| {}),
        )
        .unwrap();
        assert_eq!(
            runner.get(&job_id).unwrap().progress.status,
            CreatorJobStatus::Interrupted
        );
        let receipt = runner.resume(&job_id).unwrap();
        assert_eq!(receipt.job_id, job_id);
        let deadline = Instant::now() + Duration::from_secs(20);
        let done = loop {
            let job = runner.get(&job_id).unwrap();
            if matches!(
                job.progress.status,
                CreatorJobStatus::Completed
                    | CreatorJobStatus::Failed
                    | CreatorJobStatus::Cancelled
            ) {
                break job;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            done.progress.status,
            CreatorJobStatus::Completed,
            "{:?}",
            done.progress.error
        );
        assert_eq!(done.report_ids.len(), 2);
        let store = runner.store();
        let store = store.lock().unwrap();
        let r0 = project_service::report_get(&store, &done.report_ids[0]).unwrap();
        let r1 = project_service::report_get(&store, &done.report_ids[1]).unwrap();
        assert_eq!(r0.kind, CreatorReportKind::StructureIntegrity);
        assert_eq!(r1.kind, CreatorReportKind::CleanInstall);
        assert_eq!(
            project_service::list(&store).unwrap()[0].verification_status,
            CreatorReportStatus::Passed
        );
    }

    // 2. Corrupt CAS ZIP before resume -> worker re-audit rejects with IntegrityViolation.
    {
        let (dir, job_id) = spawn_interrupted_verify();
        let store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
        let release = project_service::release_list(&store, "build-fixture")
            .unwrap()
            .pop()
            .unwrap();
        let runner = production_runner(
            Arc::new(Mutex::new(store)),
            &dir.path().join("creator"),
            Arc::new(|_| {}),
        )
        .unwrap();
        flip_cas_zip_byte(&dir.path().join("creator"), &release);
        let receipt = runner.resume(&job_id).unwrap();
        assert_eq!(receipt.job_id, job_id);
        let deadline = Instant::now() + Duration::from_secs(20);
        let job = loop {
            let job = runner.get(&job_id).unwrap();
            if matches!(
                job.progress.status,
                CreatorJobStatus::Completed
                    | CreatorJobStatus::Failed
                    | CreatorJobStatus::Cancelled
            ) {
                break job;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(job.progress.status, CreatorJobStatus::Failed);
        assert_eq!(
            job.progress.error.unwrap().code,
            ErrorCode::IntegrityViolation
        );
        assert_eq!(job.report_ids.len(), 1);
        let store = runner.store();
        let store = store.lock().unwrap();
        assert_eq!(
            project_service::list(&store).unwrap()[0].verification_status,
            CreatorReportStatus::NotTested
        );
    }

    // 3. Interrupted Verify with 0 checkpoints refuses resume with SnapshotConflict.
    {
        let fixture = Fixture::new(managed(true), true);
        let built = fixture.terminal(&fixture.start().unwrap().job_id);
        let release = built.progress.release_id.unwrap();
        let mut req = fixture.request.clone();
        req.release_id = Some(release);
        let zero_job_id = "job-interrupted-zero".to_owned();
        let job_state = crate::fusion_creator::CreatorJob {
            progress: crate::fusion_creator::CreatorJobProgress {
                job_id: zero_job_id.clone(),
                project_id: fixture.request.project_id.clone(),
                project_revision: 1,
                sequence: 1,
                kind: CreatorJobKind::Verify,
                status: CreatorJobStatus::Interrupted,
                phase: "interrupted".into(),
                completed_units: 0,
                total_units: None,
                checkpoint_id: None,
                release_id: req.release_id.clone(),
                error: None,
            },
            checkpoints: vec![],
            report_ids: vec![],
        };
        let workspace = fixture
            ._dir
            .path()
            .join("creator")
            .join("workspaces")
            .join(blake3::hash(b"build-fixture").to_hex().as_str());
        fixture.runner.store().lock().unwrap().conn.execute(
            "INSERT INTO creator_jobs(job_id,project_id,project_revision,workspace,status,run_token,state_json,request_json,stamp_json) VALUES(?1,?2,1,?3,'interrupted','tok',?4,?5,?6)",
            rusqlite::params![
                zero_job_id,
                "build-fixture",
                workspace.to_string_lossy(),
                serde_json::to_vec(&job_state).unwrap(),
                serde_json::to_vec(&req).unwrap(),
                serde_json::to_vec(&serde_json::json!({"projectRevision":1,"inputHashes":{},"executor":{"id":"fusion.declarative","version":"1.0"}})).unwrap(),
            ],
        ).unwrap();
        assert_eq!(
            fixture.runner.resume(&zero_job_id).unwrap_err().code,
            ErrorCode::SnapshotConflict
        );
    }
}
