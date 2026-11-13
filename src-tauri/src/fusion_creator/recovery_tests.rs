//! Recovery acceptance tests: real SQLite, synced files, and an externally killed test process.
//! This entire module (including its synthetic host handler) must only be registered under cfg(test).
#![cfg(test)]

use super::jobs::{CheckpointOutput, JobContext, JobHandler, JobRunner, ResumePoint};
use super::*;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::CreatorProgramProject;
use crate::storage::RepositoryStore;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHILD_TEST: &str = "fusion_creator::recovery_tests::recovery_child_process";
const CHILD_ROOT: &str = "FUSION_CREATOR_RECOVERY_TEST_ROOT";
const CHILD_BOUNDARY: &str = "FUSION_CREATOR_RECOVERY_TEST_BOUNDARY";
const OLD_RELEASE: &str = "recovery-old-release";
const NEW_RELEASE: &str = "recovery-new-release";
const ARTIFACT: &str = "recovery-artifact";
const OLD_BYTES: &[u8] = b"previous immutable synthetic release: do not replace";
const ARTIFACT_BYTES: &[u8] = b"complete synthetic artifact, independently hashed local bytes";
type Events = Arc<Mutex<Vec<CreatorJobProgress>>>;

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn project(revision: u64) -> CreatorProgramProject {
    let mut fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
    fixture["project"]["revision"] = revision.into();
    serde_json::from_value(fixture["project"].clone()).unwrap()
}

fn request() -> CreatorJobStartRequest {
    CreatorJobStartRequest {
        project_id: project(1).project_id,
        project_revision: 1,
        build_profile_ids: vec!["windowed".into()],
        controls_profile_ids: vec!["native".into()],
        release_id: None,
        target_directory: None,
        source_url: None,
    }
}

fn boundaries() -> [CreatorCheckpointBoundary; 5] {
    [
        CreatorCheckpointBoundary::ScanClassificationSaved,
        CreatorCheckpointBoundary::ArtifactCommitted,
        CreatorCheckpointBoundary::ReleaseBuilt,
        CreatorCheckpointBoundary::VerificationCompleted,
        CreatorCheckpointBoundary::ExportCompleted,
    ]
}

fn workspace(root: &Path) -> PathBuf {
    root.join("workspaces")
        .join(hash(project(1).project_id.as_bytes()))
}

fn write_synced(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

fn publish_file(path: &Path, bytes: &[u8]) {
    let partial = path.with_file_name(format!(
        "{}.partial",
        path.file_name().unwrap().to_str().unwrap()
    ));
    write_synced(&partial, bytes);
    assert!(
        !path.exists(),
        "a committed boundary must not be executed again"
    );
    fs::rename(&partial, path).unwrap();
    // File bytes are synced before publication; this test models process death, not power loss.
    #[cfg(unix)]
    File::open(path.parent().unwrap())
        .unwrap()
        .sync_all()
        .unwrap();
}

fn manifest_bytes() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "synthetic_test_only": true,
        "project_id": project(1).project_id,
        "project_revision": 1,
        "artifact": { "id": ARTIFACT, "blake3": hash(ARTIFACT_BYTES) }
    }))
    .unwrap()
}

fn release() -> CreatorRelease {
    CreatorRelease {
        release_id: NEW_RELEASE.into(),
        project_id: project(1).project_id,
        project_revision: 1,
        manifest_blake3: hash(&manifest_bytes()),
        artifact_hashes: BTreeMap::from([(ARTIFACT.into(), hash(ARTIFACT_BYTES))]),
        report_ids: vec![],
    }
}

fn report() -> CreatorReport {
    CreatorReport {
        report_id: "recovery-structure-report".into(),
        release_id: NEW_RELEASE.into(),
        release_blake3: release().manifest_blake3,
        project_revision: 1,
        kind: CreatorReportKind::StructureIntegrity,
        status: CreatorReportStatus::Passed,
        executor: project(1).program_descriptor.executor,
        build_profile_id: "windowed".into(),
        controls_profile_id: "native".into(),
        environment: BTreeMap::from([(
            "fixture".into(),
            "synthetic, no application launched".into(),
        )]),
        diagnostics: vec![],
    }
}

// Each boundary has a complete, concrete result, not just a sentinel saying "complete".
fn boundary_files(index: usize) -> BTreeMap<String, Vec<u8>> {
    match index {
        0 => BTreeMap::from([
            (
                "scan.json".into(),
                br#"{"files":["artifact.bin"],"scan_complete":true}"#.to_vec(),
            ),
            (
                "classification.json".into(),
                br#"{"revision":1,"classification_complete":true,"role":"data"}"#.to_vec(),
            ),
        ]),
        1 => BTreeMap::from([("artifact.bin".into(), ARTIFACT_BYTES.to_vec())]),
        2 => BTreeMap::from([
            ("manifest.json".into(), manifest_bytes()),
            (
                "artifact-inventory.json".into(),
                serde_json::to_vec(&release().artifact_hashes).unwrap(),
            ),
        ]),
        3 => BTreeMap::from([("report.json".into(), serde_json::to_vec(&report()).unwrap())]),
        4 => {
            let export = serde_json::to_vec(&serde_json::json!({
                "synthetic_test_only": true,
                "manifest": manifest_bytes(),
                "artifact": ARTIFACT_BYTES,
                "report": report(),
            }))
            .unwrap();
            BTreeMap::from([
                ("export.bin".into(), export.clone()),
                (
                    "export-inventory.json".into(),
                    serde_json::to_vec(&BTreeMap::from([("export.bin", hash(&export))])).unwrap(),
                ),
            ])
        }
        _ => panic!("unknown durable boundary"),
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Evidence {
    boundary: CreatorCheckpointBoundary,
    files: BTreeMap<String, String>,
}

fn evidence(index: usize) -> Vec<u8> {
    serde_json::to_vec(&Evidence {
        boundary: boundaries()[index].clone(),
        files: boundary_files(index)
            .into_iter()
            .map(|(name, bytes)| (name, hash(&bytes)))
            .collect(),
    })
    .unwrap()
}

fn output(index: usize, bytes: Vec<u8>) -> CheckpointOutput {
    CheckpointOutput {
        boundary: boundaries()[index].clone(),
        evidence: bytes,
        artifact_id: (index == 1).then(|| ARTIFACT.into()),
        release: (index == 2).then(release),
        report: (index == 3).then(report),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Ready {
    job_id: String,
    checkpoint_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResumedFrom {
    checkpoint_id: String,
    boundary: CreatorCheckpointBoundary,
    evidence_blake3: String,
}

/// Host-owned cfg(test) implementation only. It never invokes a product executable or Packer.
struct FileHandler {
    root: PathBuf,
    pause_after: Option<usize>,
    release_gate: Arc<AtomicBool>,
    invalid_scan: Option<&'static str>,
    runs: AtomicUsize,
    validations: AtomicUsize,
    capture: Option<Arc<Mutex<Option<JobContext>>>>,
}

impl FileHandler {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            pause_after: None,
            release_gate: Arc::new(AtomicBool::new(false)),
            invalid_scan: None,
            runs: AtomicUsize::new(0),
            validations: AtomicUsize::new(0),
            capture: None,
        }
    }
}

impl JobHandler for FileHandler {
    fn input_hashes(
        &self,
        project: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        // Hash actual local fixture files on every call, never descriptor/upstream expected pins.
        let read_hash = |path: PathBuf| {
            fs::read(path)
                .map(|bytes| hash(&bytes))
                .map_err(|_| super::jobs::io())
        };
        let mut hashes = BTreeMap::from([(
            "executor_implementation".into(),
            read_hash(self.root.join("inputs/executor.bin"))?,
        )]);
        for component in &project.program_descriptor.components {
            hashes.insert(
                format!("component:{}", component.id),
                read_hash(
                    self.root
                        .join("inputs")
                        .join(format!("{}.bin", component.id)),
                )?,
            );
        }
        Ok(hashes)
    }

    fn validate_checkpoint(
        &self,
        checkpoint: &CreatorCheckpoint,
        bytes: &[u8],
    ) -> Result<bool, FusionError> {
        self.validations.fetch_add(1, Ordering::SeqCst);
        let Some(index) = boundaries()
            .iter()
            .position(|boundary| boundary == &checkpoint.boundary)
        else {
            return Ok(false);
        };
        let Ok(parsed) = serde_json::from_slice::<Evidence>(bytes) else {
            return Ok(false);
        };
        let expected: Evidence = serde_json::from_slice(&evidence(index)).unwrap();
        if parsed != expected || checkpoint.result_blake3 != hash(bytes) {
            return Ok(false);
        }
        // Exact file sets enforce scan AND classification, complete inventories/report/export,
        // and exclude partial/traversal paths. Re-read real published bytes during resume too.
        for (name, expected_bytes) in boundary_files(index) {
            if name.ends_with(".partial")
                || fs::read(workspace(&self.root).join(name)).ok().as_deref()
                    != Some(expected_bytes.as_slice())
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn run(&self, context: JobContext, resume: Option<ResumePoint>) -> Result<(), FusionError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if let Some(capture) = &self.capture {
            *capture.lock().unwrap() = Some(context.clone());
        }
        assert_eq!(context.workspace(), workspace(&self.root));
        assert_eq!(context.project().revision, 1);
        assert_eq!(context.request().project_revision, 1);
        let start = if let Some(point) = resume {
            assert!(self.validate_checkpoint(&point.checkpoint, &point.evidence)?);
            let index = boundaries()
                .iter()
                .position(|boundary| boundary == &point.checkpoint.boundary)
                .unwrap();
            publish_file(
                &self.root.join("resumed-from.json"),
                &serde_json::to_vec(&ResumedFrom {
                    checkpoint_id: point.checkpoint.checkpoint_id,
                    boundary: point.checkpoint.boundary,
                    evidence_blake3: hash(&point.evidence),
                })
                .unwrap(),
            );
            index + 1
        } else {
            0
        };
        if let Some(mode) = self.invalid_scan {
            let (name, bytes) = if mode == "partial" {
                ("scan.json.partial", b"unfinished scan".to_vec())
            } else {
                ("scan.json", boundary_files(0).remove("scan.json").unwrap())
            };
            write_synced(&context.workspace().join(name), &bytes);
            let incomplete = serde_json::to_vec(&Evidence {
                boundary: CreatorCheckpointBoundary::ScanClassificationSaved,
                files: BTreeMap::from([(name.into(), hash(&bytes))]),
            })
            .unwrap();
            context.checkpoint(output(0, incomplete))?;
            panic!("runner accepted an incomplete scan/partial file as a checkpoint");
        }
        for index in start..boundaries().len() {
            context.blocking_library(|context| {
                // create_new is a replay tripwire: resumption must not redo previous boundaries.
                write_synced(
                    &context.workspace().join(format!("stage-{index}.executed")),
                    b"executed once",
                );
                for (name, bytes) in boundary_files(index) {
                    publish_file(&context.workspace().join(name), &bytes);
                }
                Ok(())
            })?;
            let checkpoint = context.checkpoint(output(index, evidence(index)))?;
            if self.pause_after == Some(index) {
                write_synced(
                    &context
                        .workspace()
                        .join(format!("uncommitted-next-{index}.partial")),
                    b"not a checkpoint, even if left by a dead writer",
                );
                // Published only AFTER checkpoint() returns (the SQLite transaction committed).
                publish_file(
                    &self.root.join(format!("ready-{index}.json")),
                    &serde_json::to_vec(&Ready {
                        job_id: checkpoint.job_id,
                        checkpoint_id: checkpoint.checkpoint_id,
                    })
                    .unwrap(),
                );
                let deadline = Instant::now() + Duration::from_secs(60);
                while !self.release_gate.load(Ordering::SeqCst) {
                    // Deliberately no cancel polling yet: ACK is impossible until this gate opens.
                    if Instant::now() >= deadline {
                        return Err(super::jobs::io());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                context.check_cancelled()?;
            }
        }
        Ok(())
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    old_row: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_synced(
            &root.join("inputs/executor.bin"),
            b"synthetic executor implementation version one",
        );
        for component in project(1).program_descriptor.components {
            write_synced(
                &root.join("inputs").join(format!("{}.bin", component.id)),
                format!("synthetic local component {} version one", component.id).as_bytes(),
            );
        }
        write_synced(&root.join("immutable/old-release.bin"), OLD_BYTES);
        let mut store = RepositoryStore::open(&root.join("library.db")).unwrap();
        project_storage::save_project_bytes(
            &mut store,
            &serde_json::to_vec(&project(1)).unwrap(),
            None,
        )
        .unwrap();
        let runner = JobRunner::new(Arc::new(Mutex::new(store)), root, Arc::new(|_| {})).unwrap();
        let old_row = serde_json::to_vec(&CreatorRelease {
            release_id: OLD_RELEASE.into(),
            project_id: project(1).project_id,
            project_revision: 0,
            manifest_blake3: hash(OLD_BYTES),
            artifact_hashes: BTreeMap::from([("old-file".into(), hash(OLD_BYTES))]),
            report_ids: vec![],
        })
        .unwrap();
        runner.store().lock().unwrap().conn.execute(
            "INSERT INTO creator_releases(release_id,project_id,project_revision,release_json) VALUES(?1,?2,0,?3)",
            params![OLD_RELEASE, project(1).project_id, old_row],
        ).unwrap();
        drop(runner);
        Self { dir, old_row }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn open_runner(&self) -> (JobRunner, Events) {
        open_runner(self.root())
    }

    fn assert_old_unchanged(&self) {
        assert_eq!(
            hash(&fs::read(self.root().join("immutable/old-release.bin")).unwrap()),
            hash(OLD_BYTES)
        );
        let conn = self.connection();
        let (count, bytes): (i64, Vec<u8>) = conn
            .query_row(
                "SELECT count(*),release_json FROM creator_releases WHERE release_id=?1",
                [OLD_RELEASE],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(bytes, self.old_row);
        assert!(conn
            .execute(
                "UPDATE creator_releases SET release_json=?1 WHERE release_id=?2",
                params![b"replaced".as_slice(), OLD_RELEASE]
            )
            .is_err());
        assert!(conn
            .execute(
                "DELETE FROM creator_releases WHERE release_id=?1",
                [OLD_RELEASE]
            )
            .is_err());
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }

    fn connection(&self) -> Connection {
        let conn = Connection::open(self.root().join("library.db")).unwrap();
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        conn
    }
}

fn open_runner(root: &Path) -> (JobRunner, Events) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let store = RepositoryStore::open(&root.join("library.db")).unwrap();
    let runner = JobRunner::new(
        Arc::new(Mutex::new(store)),
        root,
        Arc::new(move |event| sink.lock().unwrap().push(event)),
    )
    .unwrap();
    (runner, events)
}

fn terminal(runner: &JobRunner, id: &str) -> CreatorJob {
    let deadline = Instant::now() + Duration::from_secs(15);
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

fn read_job(conn: &Connection, id: &str) -> CreatorJob {
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT state_json FROM creator_jobs WHERE job_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn count(conn: &Connection, table: &str) -> i64 {
    assert!(matches!(
        table,
        "creator_jobs" | "creator_checkpoint_results" | "creator_releases" | "creator_reports"
    ));
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn checkpoint_evidence(conn: &Connection, checkpoint_id: &str) -> Vec<u8> {
    conn.query_row(
        "SELECT evidence FROM creator_checkpoint_results WHERE checkpoint_id=?1",
        [checkpoint_id],
        |row| row.get(0),
    )
    .unwrap()
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct ReleaseOnDrop(Arc<AtomicBool>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Killed {
    fixture: Fixture,
    before: CreatorJob,
    evidence: Vec<u8>,
}

fn kill_after_checkpoint(index: usize) -> Killed {
    let fixture = Fixture::new();
    let log = File::create(fixture.root().join("child.log")).unwrap();
    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(CHILD_ROOT, fixture.root())
            .env(CHILD_BOUNDARY, index.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let ready = loop {
        if let Ok(bytes) = fs::read(fixture.root().join(format!("ready-{index}.json"))) {
            break serde_json::from_slice::<Ready>(&bytes).unwrap();
        }
        let exited = child.0.try_wait().unwrap();
        assert!(
            exited.is_none() && Instant::now() < deadline,
            "child exited/timed out before durable checkpoint: {exited:?}; log: {}",
            fs::read_to_string(fixture.root().join("child.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    // Independent connection observes committed evidence while the child is still alive.
    let conn = fixture.connection();
    let before = read_job(&conn, &ready.job_id);
    assert_eq!(before.progress.status, CreatorJobStatus::Running);
    assert_eq!(before.checkpoints.len(), index + 1);
    assert_eq!(
        before.checkpoints.last().unwrap().checkpoint_id,
        ready.checkpoint_id
    );
    assert_eq!(
        count(&conn, "creator_checkpoint_results"),
        (index + 1) as i64
    );
    let bytes = checkpoint_evidence(&conn, &ready.checkpoint_id);
    assert_eq!(bytes, evidence(index));
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "must kill a live worker, not observe a natural exit"
    );
    fixture.assert_old_unchanged();
    // A competing UI/session must not recover/interrupt the still-running writer.
    let store = RepositoryStore::open(&fixture.root().join("library.db")).unwrap();
    let competing = JobRunner::new(
        Arc::new(Mutex::new(store)),
        fixture.root(),
        Arc::new(|_| {}),
    );
    assert_eq!(
        competing
            .err()
            .expect("second session acquired the OS lease")
            .code,
        ErrorCode::OperationInProgress
    );
    assert_eq!(
        read_job(&conn, &ready.job_id).progress.sequence,
        before.progress.sequence
    );
    child.0.kill().unwrap();
    assert!(
        !child.0.wait().unwrap().success(),
        "kill must not be a graceful test exit"
    );
    assert_eq!(
        read_job(&conn, &ready.job_id).progress.status,
        CreatorJobStatus::Running
    );
    fixture.assert_old_unchanged();
    drop(conn);
    Killed {
        fixture,
        before,
        evidence: bytes,
    }
}

fn assert_interrupted(killed: &Killed, runner: &JobRunner, events: &Events) -> CreatorJob {
    let job = runner.get(&killed.before.progress.job_id).unwrap();
    assert_eq!(job.progress.status, CreatorJobStatus::Interrupted);
    assert!(job.progress.sequence > killed.before.progress.sequence);
    assert_eq!(
        serde_json::to_vec(&job.checkpoints).unwrap(),
        serde_json::to_vec(&killed.before.checkpoints).unwrap()
    );
    let events = events.lock().unwrap();
    assert_eq!(
        events.len(),
        1,
        "restart must publish one interrupted state, not fabricated work"
    );
    assert_eq!(events[0].sequence, job.progress.sequence);
    assert_eq!(events[0].status, CreatorJobStatus::Interrupted);
    job
}

/// With no child-role environment this is a normal behavioral test, not a skipped helper.
#[tokio::test]
async fn recovery_child_process() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let index: usize = std::env::var(CHILD_BOUNDARY).unwrap().parse().unwrap();
        assert!(index < boundaries().len());
        let (runner, _) = open_runner(&root);
        let mut handler = FileHandler::new(&root);
        handler.pause_after = Some(index);
        runner
            .register(CreatorJobKind::Scan, Arc::new(handler))
            .unwrap();
        runner.start(CreatorJobKind::Scan, request()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("parent did not kill its dedicated checkpoint child");
    } else {
        let fixture = Fixture::new();
        let (runner, _) = fixture.open_runner();
        runner
            .register(
                CreatorJobKind::Scan,
                Arc::new(FileHandler::new(fixture.root())),
            )
            .unwrap();
        let receipt = runner.start(CreatorJobKind::Scan, request()).unwrap();
        let job = terminal(&runner, &receipt.job_id);
        assert_eq!(
            job.progress.status,
            CreatorJobStatus::Completed,
            "{:?}",
            job.progress.error
        );
        assert_eq!(
            job.checkpoints
                .iter()
                .map(|cp| cp.boundary.clone())
                .collect::<Vec<_>>(),
            boundaries()
        );
        assert_eq!(count(&fixture.connection(), "creator_reports"), 1);
        fixture.assert_old_unchanged();
    }
}

#[tokio::test]
async fn killed_child_recovers_and_resumes_each_of_the_five_durable_boundaries() {
    for index in 0..boundaries().len() {
        let killed = kill_after_checkpoint(index);
        let partial =
            workspace(killed.fixture.root()).join(format!("uncommitted-next-{index}.partial"));
        let partial_before = hash(&fs::read(&partial).unwrap());
        let committed_before: BTreeMap<_, _> = (0..=index)
            .flat_map(boundary_files)
            .map(|(name, _)| {
                let digest = hash(&fs::read(workspace(killed.fixture.root()).join(&name)).unwrap());
                (name, digest)
            })
            .collect();
        let (runner, events) = killed.fixture.open_runner();
        let interrupted = assert_interrupted(&killed, &runner, &events);
        let handler = Arc::new(FileHandler::new(killed.fixture.root()));
        runner
            .register(CreatorJobKind::Scan, handler.clone())
            .unwrap();
        let receipt = runner.resume(&interrupted.progress.job_id).unwrap();
        assert_eq!(
            receipt.job_id, killed.before.progress.job_id,
            "resume must retain job identity"
        );
        assert_eq!(receipt.project_id, project(1).project_id);
        assert_eq!(receipt.project_revision, 1);
        let done = terminal(&runner, &receipt.job_id);
        assert_eq!(
            done.progress.status,
            CreatorJobStatus::Completed,
            "{:?}",
            done.progress.error
        );
        assert!(done.progress.sequence > interrupted.progress.sequence);
        assert_eq!(
            done.checkpoints
                .iter()
                .map(|cp| cp.boundary.clone())
                .collect::<Vec<_>>(),
            boundaries()
        );
        assert_eq!(
            serde_json::to_vec(&done.checkpoints[..=index]).unwrap(),
            serde_json::to_vec(&killed.before.checkpoints).unwrap()
        );
        let observed: ResumedFrom = serde_json::from_slice(
            &fs::read(killed.fixture.root().join("resumed-from.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            observed.checkpoint_id,
            killed.before.checkpoints[index].checkpoint_id
        );
        assert_eq!(observed.boundary, boundaries()[index]);
        assert_eq!(observed.evidence_blake3, hash(&killed.evidence));
        assert_eq!(handler.runs.load(Ordering::SeqCst), 1);
        assert!(handler.validations.load(Ordering::SeqCst) >= 1);
        for (name, digest) in committed_before {
            assert_eq!(
                hash(&fs::read(workspace(killed.fixture.root()).join(name)).unwrap()),
                digest
            );
        }
        assert_eq!(hash(&fs::read(&partial).unwrap()), partial_before);
        let conn = killed.fixture.connection();
        assert_eq!(count(&conn, "creator_jobs"), 1);
        assert_eq!(count(&conn, "creator_checkpoint_results"), 5);
        assert_eq!(count(&conn, "creator_releases"), 2);
        assert_eq!(count(&conn, "creator_reports"), 1);
        for checkpoint in &done.checkpoints {
            let bytes = checkpoint_evidence(&conn, &checkpoint.checkpoint_id);
            let parsed: Evidence = serde_json::from_slice(&bytes).unwrap();
            assert!(parsed.files.keys().all(|name| !name.ends_with(".partial")));
            assert_eq!(hash(&bytes), checkpoint.result_blake3);
            assert!(handler.validate_checkpoint(checkpoint, &bytes).unwrap());
        }
        assert_eq!(done.report_ids, vec![report().report_id]);
        assert!(
            runner.resume(&receipt.job_id).is_err(),
            "completed jobs must not resume"
        );
        let events = events.lock().unwrap();
        assert!(events
            .iter()
            .all(|event| event.job_id == receipt.job_id && event.project_revision == 1));
        assert!(events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
        assert_eq!(events.last().unwrap().sequence, done.progress.sequence);
        assert_eq!(events.last().unwrap().status, CreatorJobStatus::Completed);
        killed.fixture.assert_old_unchanged();
    }
}

#[tokio::test]
async fn resume_reaudits_component_bytes_executor_implementation_and_project_revision() {
    for changed in ["component", "executor", "revision"] {
        let killed = kill_after_checkpoint(0);
        let (runner, events) = killed.fixture.open_runner();
        let interrupted = assert_interrupted(&killed, &runner, &events);
        let handler = Arc::new(FileHandler::new(killed.fixture.root()));
        runner
            .register(CreatorJobKind::Scan, handler.clone())
            .unwrap();
        match changed {
            "component" => {
                let id = project(1).program_descriptor.components[0].id.clone();
                fs::write(
                    killed
                        .fixture
                        .root()
                        .join("inputs")
                        .join(format!("{id}.bin")),
                    b"changed real component bytes",
                )
                .unwrap();
            }
            "executor" => fs::write(
                killed.fixture.root().join("inputs/executor.bin"),
                b"changed implementation, same ABI id/version",
            )
            .unwrap(),
            "revision" => {
                let store = runner.store();
                project_storage::save_project_bytes(
                    &mut store.lock().unwrap(),
                    &serde_json::to_vec(&project(2)).unwrap(),
                    Some(1),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = serde_json::to_vec(&interrupted).unwrap();
        let error = runner.resume(&interrupted.progress.job_id).unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::SnapshotConflict,
            "{changed}: {error:?}"
        );
        assert_eq!(
            handler.runs.load(Ordering::SeqCst),
            0,
            "{changed} dispatched a worker"
        );
        assert_eq!(
            count(&killed.fixture.connection(), "creator_jobs"),
            1,
            "resume created a replacement job"
        );
        assert_eq!(
            count(&killed.fixture.connection(), "creator_checkpoint_results"),
            1
        );
        assert_eq!(
            serde_json::to_vec(&runner.get(&interrupted.progress.job_id).unwrap()).unwrap(),
            before,
            "failed resume must leave interrupted state untouched"
        );
        assert!(!killed.fixture.root().join("resumed-from.json").exists());
        killed.fixture.assert_old_unchanged();
    }
}

#[tokio::test]
async fn resume_revalidates_real_output_and_does_not_promote_a_partial_replacement() {
    for damage in ["corrupt", "partial-replacement"] {
        let killed = kill_after_checkpoint(0);
        let (runner, events) = killed.fixture.open_runner();
        let interrupted = assert_interrupted(&killed, &runner, &events);
        let handler = Arc::new(FileHandler::new(killed.fixture.root()));
        runner
            .register(CreatorJobKind::Scan, handler.clone())
            .unwrap();
        let classification = workspace(killed.fixture.root()).join("classification.json");
        if damage == "corrupt" {
            fs::write(&classification, b"corrupt saved classification").unwrap();
        } else {
            fs::rename(
                &classification,
                classification.with_file_name("classification.json.partial"),
            )
            .unwrap();
        }
        assert!(
            runner.resume(&interrupted.progress.job_id).is_err(),
            "{damage} was reused"
        );
        assert!(
            handler.validations.load(Ordering::SeqCst) > 0,
            "resume did not call real output validation"
        );
        assert_eq!(handler.runs.load(Ordering::SeqCst), 0);
        assert_eq!(count(&killed.fixture.connection(), "creator_jobs"), 1);
        assert_eq!(
            count(&killed.fixture.connection(), "creator_checkpoint_results"),
            1
        );
        assert_eq!(
            runner
                .get(&interrupted.progress.job_id)
                .unwrap()
                .progress
                .status,
            CreatorJobStatus::Interrupted
        );
        killed.fixture.assert_old_unchanged();
    }
}

#[tokio::test]
async fn partial_files_and_scan_without_classification_never_become_checkpoints() {
    for mode in ["partial", "scan-only"] {
        let fixture = Fixture::new();
        let (runner, _) = fixture.open_runner();
        let mut handler = FileHandler::new(fixture.root());
        handler.invalid_scan = Some(mode);
        runner
            .register(CreatorJobKind::Scan, Arc::new(handler))
            .unwrap();
        let receipt = runner.start(CreatorJobKind::Scan, request()).unwrap();
        let job = terminal(&runner, &receipt.job_id);
        assert_eq!(job.progress.status, CreatorJobStatus::Failed, "{mode}");
        assert_eq!(
            job.progress.error.unwrap().code,
            ErrorCode::IntegrityViolation
        );
        assert!(job.checkpoints.is_empty());
        assert!(job.progress.checkpoint_id.is_none());
        assert_eq!(
            count(&fixture.connection(), "creator_checkpoint_results"),
            0
        );
        assert_eq!(count(&fixture.connection(), "creator_releases"), 1);
        fixture.assert_old_unchanged();
    }
}

fn wait_ready(root: &Path, index: usize) -> Ready {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(bytes) = fs::read(root.join(format!("ready-{index}.json"))) {
            return serde_json::from_slice(&bytes).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "worker did not reach checkpoint gate {index}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[tokio::test]
async fn resumed_workspace_has_one_writer_and_cancel_waits_for_worker_ack() {
    let killed = kill_after_checkpoint(0);
    let (runner, events) = killed.fixture.open_runner();
    let interrupted = assert_interrupted(&killed, &runner, &events);
    let capture = Arc::new(Mutex::new(None));
    let mut handler = FileHandler::new(killed.fixture.root());
    handler.pause_after = Some(1);
    handler.capture = Some(capture.clone());
    let release = ReleaseOnDrop(handler.release_gate.clone());
    runner
        .register(CreatorJobKind::Scan, Arc::new(handler))
        .unwrap();
    let receipt = runner.resume(&interrupted.progress.job_id).unwrap();
    let ready = wait_ready(killed.fixture.root(), 1);
    assert_eq!(ready.job_id, receipt.job_id);
    let late_context = capture.lock().unwrap().take().unwrap();
    assert_eq!(
        runner
            .start(CreatorJobKind::Scan, request())
            .unwrap_err()
            .code,
        ErrorCode::OperationInProgress
    );
    assert!(
        runner.resume(&receipt.job_id).is_err(),
        "second resume started another writer"
    );
    assert_eq!(count(&killed.fixture.connection(), "creator_jobs"), 1);
    let cancelling = runner.cancel(&receipt.job_id).unwrap();
    assert_eq!(cancelling.progress.status, CreatorJobStatus::Cancelling);
    assert!(!release.0.load(Ordering::SeqCst));
    assert_eq!(
        runner.get(&receipt.job_id).unwrap().progress.status,
        CreatorJobStatus::Cancelling,
        "cancel returned terminal before the worker acknowledged it"
    );
    assert_eq!(
        runner.cancel(&receipt.job_id).unwrap().progress.sequence,
        cancelling.progress.sequence
    );
    assert_eq!(
        runner
            .start(CreatorJobKind::Scan, request())
            .unwrap_err()
            .code,
        ErrorCode::OperationInProgress,
        "cancelling worker lost its writer lease before ACK"
    );
    killed.fixture.assert_old_unchanged();
    drop(release);
    let cancelled = terminal(&runner, &receipt.job_id);
    assert_eq!(cancelled.progress.status, CreatorJobStatus::Cancelled);
    assert!(cancelled.progress.sequence > cancelling.progress.sequence);
    assert_eq!(cancelled.checkpoints.len(), 2);
    let terminal_bytes = serde_json::to_vec(&cancelled).unwrap();
    assert!(late_context.check_cancelled().is_err());
    assert!(late_context
        .progress("late running event", 0, None)
        .is_err());
    assert!(late_context.checkpoint(output(2, evidence(2))).is_err());
    assert_eq!(
        serde_json::to_vec(&runner.get(&receipt.job_id).unwrap()).unwrap(),
        terminal_bytes
    );
    assert_eq!(count(&killed.fixture.connection(), "creator_releases"), 1);
    assert_eq!(
        count(&killed.fixture.connection(), "creator_checkpoint_results"),
        2
    );
    let events = events.lock().unwrap();
    assert!(events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    assert_eq!(events.last().unwrap().status, CreatorJobStatus::Cancelled);
    killed.fixture.assert_old_unchanged();
}
