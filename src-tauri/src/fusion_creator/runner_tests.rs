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
