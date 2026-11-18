//! Real SQLite + blocking worker tests. Synthetic handlers are cfg(test) only.
use super::jobs::*;
use super::*;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::storage::RepositoryStore;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn fixture(revision: u64) -> crate::fusion_source::program_schema::CreatorProgramProject {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
    value["project"]["revision"] = revision.into();
    serde_json::from_value(value["project"].clone()).unwrap()
}
fn setup() -> (
    tempfile::TempDir,
    JobRunner,
    Arc<Mutex<Vec<CreatorJobProgress>>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
    project_storage::save_project_bytes(
        &mut store,
        &serde_json::to_vec(&fixture(1)).unwrap(),
        None,
    )
    .unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let runner = JobRunner::new(
        Arc::new(Mutex::new(store)),
        dir.path(),
        Arc::new(move |event| sink.lock().unwrap().push(event)),
    )
    .unwrap();
    (dir, runner, events)
}
fn request() -> CreatorJobStartRequest {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
    let mut request: CreatorJobStartRequest =
        serde_json::from_value(f["jobStart"].clone()).unwrap();
    request.project_id = fixture(1).project_id;
    request.project_revision = 1;
    request.release_id = None;
    request
}
fn terminal(runner: &JobRunner, id: &str) -> CreatorJob {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let job = runner.get(id).unwrap();
        if matches!(
            job.progress.status,
            CreatorJobStatus::Completed | CreatorJobStatus::Failed | CreatorJobStatus::Cancelled
        ) {
            return job;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not terminate: {:?}",
            job.progress
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
struct Complete;
impl JobHandler for Complete {
    fn input_hashes(
        &self,
        project: &crate::fusion_source::program_schema::CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
    ) -> Result<std::collections::BTreeMap<String, String>, FusionError> {
        let hash = blake3::hash(b"test-only handler v1").to_hex().to_string();
        let mut hashes = std::collections::BTreeMap::from([
            ("executor_implementation".into(), hash.clone()),
            ("capability:eden_param_package_v1".into(), hash.clone()),
        ]);
        for component in &project.program_descriptor.components {
            hashes.insert(format!("component:{}", component.id), hash.clone());
        }
        Ok(hashes)
    }
    fn validate_checkpoint(
        &self,
        _: &CreatorCheckpoint,
        evidence: &[u8],
    ) -> Result<bool, FusionError> {
        Ok(evidence == b"complete scan and classification")
    }
    fn run(&self, context: JobContext, _: Option<ResumePoint>) -> Result<(), FusionError> {
        context.progress("scan", 1, Some(2))?;
        context.checkpoint(CheckpointOutput {
            boundary: CreatorCheckpointBoundary::ScanClassificationSaved,
            evidence: b"complete scan and classification".to_vec(),
            artifact_id: None,
            release: None,
            report: None,
        })?;
        Ok(())
    }
}

#[tokio::test]
async fn registered_handler_persists_progress_and_ignores_late_worker_events() {
    let (_dir, runner, events) = setup();
    runner
        .register(CreatorJobKind::Scan, Arc::new(Complete))
        .unwrap();
    let receipt = runner.start(CreatorJobKind::Scan, request()).unwrap();
    let job = terminal(&runner, &receipt.job_id);
    assert_eq!(job.progress.status, CreatorJobStatus::Completed);
    assert_eq!(job.checkpoints.len(), 1);
    let events = events.lock().unwrap();
    assert!(events.len() >= 4);
    assert!(events.windows(2).all(|w| w[0].sequence < w[1].sequence));
    assert_eq!(events.last().unwrap().status, CreatorJobStatus::Completed);
}

#[tokio::test]
async fn missing_production_handler_is_unavailable_and_creates_no_job() {
    let (_dir, runner, _) = setup();
    assert_eq!(
        runner
            .start(CreatorJobKind::Build, request())
            .unwrap_err()
            .code,
        ErrorCode::ActionUnavailable
    );
    let store = runner.store();
    let count: i64 = store
        .lock()
        .unwrap()
        .conn
        .query_row("SELECT count(*) FROM creator_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn explicit_missing_release_does_not_queue_a_job() {
    let (_dir, runner, events) = setup();
    runner
        .register(CreatorJobKind::Verify, Arc::new(Complete))
        .unwrap();
    let mut request = request();
    request.release_id = Some("never-built".into());
    assert_eq!(
        runner
            .start(CreatorJobKind::Verify, request)
            .unwrap_err()
            .code,
        ErrorCode::SnapshotNotFound
    );
    assert!(events.lock().unwrap().is_empty());
}

const MANIFEST: &[u8] = br#"{"schema_version":"1.1","id":"fixture-game"}"#;
const ARTIFACT: &[u8] = b"verified fixture payload";
const RELEASE: &str = "receipt-release";

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

struct BuildBundle {
    bad_hash: bool,
}
impl JobHandler for BuildBundle {
    fn input_hashes(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        Complete.input_hashes(project, bindings)
    }
    fn validate_checkpoint(&self, _: &CreatorCheckpoint, _: &[u8]) -> Result<bool, FusionError> {
        Ok(false) // Only the runner's CAS audit can authorize ReleaseBuilt.
    }
    fn run(&self, context: JobContext, _: Option<ResumePoint>) -> Result<(), FusionError> {
        let stage = context.workspace().join("build-output");
        std::fs::create_dir_all(stage.join("artifacts")).unwrap();
        std::fs::write(stage.join("manifest.json"), MANIFEST).unwrap();
        std::fs::write(stage.join("artifacts/runtime"), ARTIFACT).unwrap();
        context.checkpoint_release(
            CreatorRelease {
                release_id: RELEASE.into(),
                project_id: context.project().project_id.clone(),
                project_revision: context.project().revision,
                manifest_blake3: hash(MANIFEST),
                artifact_hashes: BTreeMap::from([(
                    "runtime".into(),
                    hash(if self.bad_hash { b"wrong" } else { ARTIFACT }),
                )]),
                report_ids: vec![],
            },
            "build-output",
        )?;
        // Destroy scratch bytes; readback must come from the published CAS copy.
        std::fs::remove_dir_all(stage).unwrap();
        Ok(())
    }
}

struct VerifyBundle;
impl JobHandler for VerifyBundle {
    fn input_hashes(
        &self,
        _: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        panic!("selected release must be available during stamping")
    }
    fn input_hashes_for_release(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        release: Option<&ReleaseContent>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        assert_eq!(release.unwrap().manifest(), MANIFEST);
        let mut hashes = Complete.input_hashes(project, bindings)?;
        hashes.insert(
            "selected_release".into(),
            hash(b"cannot override runner receipt"),
        );
        Ok(hashes)
    }
    fn validate_checkpoint(&self, _: &CreatorCheckpoint, _: &[u8]) -> Result<bool, FusionError> {
        Ok(true)
    }
    fn run(&self, context: JobContext, _: Option<ResumePoint>) -> Result<(), FusionError> {
        use std::io::Read;
        let content = context.selected_release().unwrap();
        assert_eq!(content.release().release_id, RELEASE);
        let mut bytes = Vec::new();
        content
            .open_artifact("runtime")?
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, ARTIFACT);
        context.checkpoint(CheckpointOutput {
            boundary: CreatorCheckpointBoundary::ScanClassificationSaved,
            evidence: b"fixture output".to_vec(),
            artifact_id: None,
            release: None,
            report: None,
        })?;
        Ok(())
    }
}

#[tokio::test]
async fn release_receipt_survives_scratch_and_pins_selection_for_handler() {
    let (_dir, runner, _) = setup();
    runner
        .register(
            CreatorJobKind::Build,
            Arc::new(BuildBundle { bad_hash: false }),
        )
        .unwrap();
    let built = runner.start(CreatorJobKind::Build, request()).unwrap();
    let job = terminal(&runner, &built.job_id);
    assert_eq!(
        job.progress.status,
        CreatorJobStatus::Completed,
        "{:?}",
        job.progress.error
    );
    let mut request = request();
    let release = runner
        .read_release_content(&request.project_id, 1, RELEASE)
        .unwrap();
    assert_eq!(release.manifest(), MANIFEST);
    assert_eq!(release.inventory().len(), 2);
    assert!(runner
        .read_release_content(&request.project_id, 2, RELEASE)
        .is_err());
    assert!(runner
        .read_release_content("foreign-project", 1, RELEASE)
        .is_err());
    assert_eq!(release.receipt_blake3(), job.checkpoints[0].result_blake3);

    runner
        .register(CreatorJobKind::Verify, Arc::new(VerifyBundle))
        .unwrap();
    request.release_id = Some(RELEASE.into());
    let started = runner
        .start(CreatorJobKind::Verify, request.clone())
        .unwrap();
    let verified = terminal(&runner, &started.job_id);
    assert_eq!(
        verified.progress.status,
        CreatorJobStatus::Completed,
        "{:?}",
        verified.progress.error
    );
    assert_eq!(
        verified.checkpoints[0].stamp.input_hashes["selected_release"],
        release.receipt_blake3()
    );

    // Preserve file size and original readonly state so only a byte audit catches this.
    let cas = crate::fusion_source::cas::CasStore::open(_dir.path()).unwrap();
    let object = std::fs::read_dir(cas.root().join("objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let path = object.path().join("data/artifacts/runtime");
    let original = std::fs::metadata(&path).unwrap().permissions();
    std::fs::remove_file(&path)
        .or_else(|_| {
            #[cfg(windows)]
            {
                let mut writable = original.clone();
                writable.set_readonly(false);
                std::fs::set_permissions(&path, writable)?;
            }
            std::fs::remove_file(&path)
        })
        .unwrap();
    std::fs::write(&path, vec![b'x'; ARTIFACT.len()]).unwrap();
    std::fs::set_permissions(&path, original).unwrap();
    assert!(release.open_artifact("runtime").is_err());
    assert_eq!(
        runner
            .start(CreatorJobKind::Verify, request)
            .unwrap_err()
            .code,
        ErrorCode::IntegrityViolation
    );
}

#[tokio::test]
async fn wrong_artifact_digest_never_commits_release_or_checkpoint() {
    let (_dir, runner, _) = setup();
    runner
        .register(
            CreatorJobKind::Build,
            Arc::new(BuildBundle { bad_hash: true }),
        )
        .unwrap();
    let started = runner.start(CreatorJobKind::Build, request()).unwrap();
    let job = terminal(&runner, &started.job_id);
    assert_eq!(job.progress.status, CreatorJobStatus::Failed);
    assert_eq!(
        job.progress.error.unwrap().code,
        ErrorCode::IntegrityViolation
    );
    assert!(job.checkpoints.is_empty());
    assert!(job.progress.release_id.is_none());
    assert_eq!(
        runner
            .read_release_content(&started.project_id, 1, RELEASE)
            .err()
            .unwrap()
            .code,
        ErrorCode::SnapshotNotFound
    );
}

#[tokio::test]
async fn metadata_only_release_cannot_authorize_selected_job() {
    let (_dir, runner, events) = setup();
    let mut request = request();
    let release = CreatorRelease {
        release_id: RELEASE.into(),
        project_id: request.project_id.clone(),
        project_revision: request.project_revision,
        manifest_blake3: hash(MANIFEST),
        artifact_hashes: BTreeMap::from([("runtime".into(), hash(ARTIFACT))]),
        report_ids: vec![],
    };
    runner.store().lock().unwrap().conn.execute(
        "INSERT INTO creator_releases(release_id,project_id,project_revision,release_json) VALUES (?1,?2,?3,?4)",
        rusqlite::params![release.release_id, release.project_id, release.project_revision, serde_json::to_vec(&release).unwrap()],
    ).unwrap();
    runner
        .register(CreatorJobKind::Verify, Arc::new(VerifyBundle))
        .unwrap();
    request.release_id = Some(RELEASE.into());
    assert_eq!(
        runner
            .start(CreatorJobKind::Verify, request)
            .unwrap_err()
            .code,
        ErrorCode::IntegrityViolation
    );
    assert!(events.lock().unwrap().is_empty());
}
