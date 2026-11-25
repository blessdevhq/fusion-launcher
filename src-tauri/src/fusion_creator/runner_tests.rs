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
struct PausedAudit {
    entered: std::sync::mpsc::SyncSender<()>,
    resume: Mutex<std::sync::mpsc::Receiver<()>>,
    observed: std::sync::mpsc::SyncSender<Option<ErrorCode>>,
    packing_entered: Arc<std::sync::atomic::AtomicBool>,
}
impl JobHandler for PausedAudit {
    fn deferred_input_audit(&self) -> bool {
        true
    }
    fn input_hashes(
        &self,
        _: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        panic!("audit must not block the receipt");
    }
    fn input_hashes_checked(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        _: Option<&ReleaseContent>,
        check: &dyn Fn() -> Result<(), FusionError>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        self.entered.send(()).unwrap();
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let result = check();
        self.observed
            .send(result.as_ref().err().map(|error| error.code))
            .unwrap();
        result?;
        Complete.input_hashes(project, bindings)
    }
    fn validate_checkpoint(&self, _: &CreatorCheckpoint, _: &[u8]) -> Result<bool, FusionError> {
        Ok(false)
    }
    fn run(&self, _: JobContext, _: Option<ResumePoint>) -> Result<(), FusionError> {
        self.packing_entered
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::test]
async fn deferred_input_audit_can_be_cancelled_using_the_returned_receipt() {
    let (_dir, runner, _) = setup();
    let (entered, waiting) = std::sync::mpsc::sync_channel(1);
    let (resume, barrier) = std::sync::mpsc::sync_channel(1);
    let (observed, outcome) = std::sync::mpsc::sync_channel(1);
    let packing_entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    runner
        .register(
            CreatorJobKind::Build,
            Arc::new(PausedAudit {
                entered,
                resume: Mutex::new(barrier),
                observed,
                packing_entered: packing_entered.clone(),
            }),
        )
        .unwrap();
    let receipt = runner.start(CreatorJobKind::Build, request()).unwrap();
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        runner.cancel(&receipt.job_id).unwrap().progress.status,
        CreatorJobStatus::Cancelling
    );
    resume.send(()).unwrap();
    assert_eq!(
        outcome.recv_timeout(Duration::from_secs(5)).unwrap(),
        Some(ErrorCode::Cancelled)
    );
    let job = terminal(&runner, &receipt.job_id);
    assert_eq!(job.progress.status, CreatorJobStatus::Cancelled);
    assert!(job.checkpoints.is_empty());
    assert!(!packing_entered.load(std::sync::atomic::Ordering::Relaxed));
}

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
        let result = context.checkpoint_release(
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
        );
        // Destroy scratch bytes; readback must come from the published CAS copy.
        std::fs::remove_dir_all(stage).unwrap();
        result?;
        Ok(())
    }
}

#[tokio::test]
async fn cancelled_release_capture_commits_neither_checkpoint_nor_release() {
    use crate::fusion_source::cas::{directory_digest, CasStore};
    use std::fs;
    let (dir, runner, _) = setup();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir(input.path().join("artifacts")).unwrap();
    fs::write(input.path().join("manifest.json"), MANIFEST).unwrap();
    fs::write(input.path().join("artifacts/runtime"), ARTIFACT).unwrap();
    let digest = directory_digest(input.path(), &|| Ok(())).unwrap();
    let cas = CasStore::open(dir.path()).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(cas.root().join("locks").join(format!("{digest}.lock")))
        .unwrap();
    fs2::FileExt::lock_exclusive(&held).unwrap();
    runner
        .register(
            CreatorJobKind::Build,
            Arc::new(BuildBundle { bad_hash: false }),
        )
        .unwrap();
    let receipt = runner.start(CreatorJobKind::Build, request()).unwrap();
    let staging = cas.root().join("staging");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_dir(&staging)
            .unwrap()
            .any(|entry| entry.unwrap().path().join("data/manifest.json").is_file())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "release capture did not reach CAS staging"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        runner.cancel(&receipt.job_id).unwrap().progress.status,
        CreatorJobStatus::Cancelling
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let job = loop {
        let job = runner.get(&receipt.job_id).unwrap();
        if matches!(
            job.progress.status,
            CreatorJobStatus::Cancelled | CreatorJobStatus::Failed | CreatorJobStatus::Completed
        ) {
            break job;
        }
        assert!(
            Instant::now() < deadline,
            "cancellation must finish before the competing writer unlocks"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    drop(held);
    assert_eq!(job.progress.status, CreatorJobStatus::Cancelled);
    assert!(job.progress.release_id.is_none());
    assert!(job.checkpoints.is_empty());
    let store = runner.store();
    let store = store.lock().unwrap();
    let count: usize = store.conn.query_row(
        "SELECT (SELECT COUNT(*) FROM creator_releases) + (SELECT COUNT(*) FROM creator_checkpoint_results)", [], |row| row.get(0)
    ).unwrap();
    assert_eq!(count, 0);
    assert_eq!(fs::read_dir(staging).unwrap().count(), 0);
    assert!(!cas.root().join("objects").join(digest).exists());
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
                // Windows clears FILE_ATTRIBUTE_READONLY without changing the ACL.
                #[allow(clippy::permissions_set_readonly_false)]
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

const DEFERRED_RESUME_CHILD: &str = "FUSION_CREATOR_DEFERRED_RESUME_CHILD";

struct DeferredResumeHandler {
    entered: Option<std::sync::mpsc::SyncSender<()>>,
    resume: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
    observed: Option<std::sync::mpsc::SyncSender<Option<ErrorCode>>>,
    run_entered: Arc<std::sync::atomic::AtomicBool>,
}

impl JobHandler for DeferredResumeHandler {
    fn deferred_input_audit(&self) -> bool {
        true
    }
    fn input_hashes(
        &self,
        _: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        panic!(
            "deferred resume must re-audit inside the worker, not synchronously on start/resume"
        );
    }
    fn input_hashes_checked(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        _: Option<&ReleaseContent>,
        check: &dyn Fn() -> Result<(), FusionError>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        if let Some(entered) = &self.entered {
            let _ = entered.send(());
        }
        if let Some(resume) = &self.resume {
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        let result = check();
        if let Some(observed) = &self.observed {
            let _ = observed.send(result.as_ref().err().map(|error| error.code));
        }
        result?;
        Complete.input_hashes(project, bindings)
    }
    fn validate_checkpoint(
        &self,
        checkpoint: &CreatorCheckpoint,
        evidence: &[u8],
    ) -> Result<bool, FusionError> {
        Ok(
            checkpoint.boundary == CreatorCheckpointBoundary::ScanClassificationSaved
                && evidence == b"complete scan and classification",
        )
    }
    fn run(&self, context: JobContext, resume: Option<ResumePoint>) -> Result<(), FusionError> {
        self.run_entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(point) = resume {
            assert_eq!(
                point.checkpoint.boundary,
                CreatorCheckpointBoundary::ScanClassificationSaved
            );
            context.progress("resumed", 2, Some(2))?;
            return Ok(());
        }
        Complete.run(context, None)
    }
}

#[tokio::test]
async fn deferred_resume_reaudits_in_worker_and_can_be_cancelled_from_receipt() {
    if let Some(root) = std::env::var_os(DEFERRED_RESUME_CHILD) {
        let root = std::path::PathBuf::from(root);
        let mut store = RepositoryStore::open(&root.join("library.db")).unwrap();
        project_storage::save_project_bytes(
            &mut store,
            &serde_json::to_vec(&fixture(1)).unwrap(),
            None,
        )
        .unwrap();
        let runner = JobRunner::new(
            Arc::new(Mutex::new(store)),
            &root,
            Arc::new(|progress| {
                if progress.checkpoint_id.is_some() {
                    std::process::exit(99);
                }
            }),
        )
        .unwrap();
        runner
            .register(
                CreatorJobKind::Scan,
                Arc::new(DeferredResumeHandler {
                    entered: None,
                    resume: None,
                    observed: None,
                    run_entered: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                }),
            )
            .unwrap();
        runner.start(CreatorJobKind::Scan, request()).unwrap();
        std::thread::sleep(Duration::from_secs(15));
        panic!("child should have exited at checkpoint commit");
    }

    let spawn_interrupted = || -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fusion_creator::runner_tests::deferred_resume_reaudits_in_worker_and_can_be_cancelled_from_receipt",
                "--nocapture",
            ])
            .env(DEFERRED_RESUME_CHILD, dir.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(99));
        let store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
        let job_id: String = store
            .conn
            .query_row("SELECT job_id FROM creator_jobs", [], |r| r.get(0))
            .unwrap();
        (dir, job_id)
    };

    // 1. Cancel during deferred worker re-audit using the receipt returned by resume().
    {
        let (dir, job_id) = spawn_interrupted();
        let store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
        let runner =
            JobRunner::new(Arc::new(Mutex::new(store)), dir.path(), Arc::new(|_| {})).unwrap();
        assert_eq!(
            runner.get(&job_id).unwrap().progress.status,
            CreatorJobStatus::Interrupted
        );
        let (entered, waiting) = std::sync::mpsc::sync_channel(1);
        let (resume_tx, barrier) = std::sync::mpsc::sync_channel(1);
        let (observed, outcome) = std::sync::mpsc::sync_channel(1);
        let run_entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        runner
            .register(
                CreatorJobKind::Scan,
                Arc::new(DeferredResumeHandler {
                    entered: Some(entered),
                    resume: Some(Mutex::new(barrier)),
                    observed: Some(observed),
                    run_entered: run_entered.clone(),
                }),
            )
            .unwrap();
        let receipt = runner.resume(&job_id).unwrap();
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            runner.cancel(&receipt.job_id).unwrap().progress.status,
            CreatorJobStatus::Cancelling
        );
        resume_tx.send(()).unwrap();
        assert_eq!(
            outcome.recv_timeout(Duration::from_secs(5)).unwrap(),
            Some(ErrorCode::Cancelled)
        );
        let job = terminal(&runner, &receipt.job_id);
        assert_eq!(job.progress.status, CreatorJobStatus::Cancelled);
        assert!(!run_entered.load(std::sync::atomic::Ordering::SeqCst));
    }

    // 2. Happy path deferred resume re-audits in worker and completes.
    {
        let (dir, job_id) = spawn_interrupted();
        let store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
        let runner =
            JobRunner::new(Arc::new(Mutex::new(store)), dir.path(), Arc::new(|_| {})).unwrap();
        let run_entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        runner
            .register(
                CreatorJobKind::Scan,
                Arc::new(DeferredResumeHandler {
                    entered: None,
                    resume: None,
                    observed: None,
                    run_entered: run_entered.clone(),
                }),
            )
            .unwrap();
        let receipt = runner.resume(&job_id).unwrap();
        let job = terminal(&runner, &receipt.job_id);
        assert_eq!(
            job.progress.status,
            CreatorJobStatus::Completed,
            "{:?}",
            job.progress.error
        );
        assert!(run_entered.load(std::sync::atomic::Ordering::SeqCst));
    }
}
