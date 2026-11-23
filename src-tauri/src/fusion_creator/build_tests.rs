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
        let error = fixture.start().err().expect("unsupported Build succeeded");
        assert_eq!(error.code, code);
        assert!(fixture.releases().is_empty());
    }
}
