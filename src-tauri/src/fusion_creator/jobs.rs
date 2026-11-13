//! Durable Creator runner. Only trusted host handlers execute work; production starts empty.
//! A worker owns its session/workspace leases independently of the IPC/UI lifetime.
use super::*;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety::{self, DirectoryGuard};
use crate::fusion_source::program_schema::{CreatorProgramProject, ProgramControlsProfile};
use crate::storage::RepositoryStore;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

pub(crate) type ProgressSink = Arc<dyn Fn(CreatorJobProgress) + Send + Sync>;

/// G implements this host-only API; never accept implementations from a Source.
/// input_hashes MUST audit actual inputs (not echo expected pins), retaining immutable
/// inputs/read leases for the operation. Include component:<id>, executor_implementation,
/// and capability:<id> implementation digests. Called again before reuse/publication.
pub trait JobHandler: Send + Sync {
    fn input_hashes(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError>;
    /// Validate complete, already atomically published output against its real storage.
    /// Scan requires BOTH scan and classification; a partial file is never evidence.
    fn validate_checkpoint(
        &self,
        checkpoint: &CreatorCheckpoint,
        evidence: &[u8],
    ) -> Result<bool, FusionError>;
    fn run(&self, context: JobContext, resume: Option<ResumePoint>) -> Result<(), FusionError>;
}

#[derive(Clone)]
pub struct ResumePoint {
    pub checkpoint: CreatorCheckpoint,
    pub evidence: Vec<u8>,
}

pub struct CheckpointOutput {
    pub boundary: CreatorCheckpointBoundary,
    pub evidence: Vec<u8>,
    pub artifact_id: Option<String>,
    pub release: Option<CreatorRelease>,
    pub report: Option<CreatorReport>,
}

#[derive(Clone)]
pub struct JobRunner(Arc<Inner>);
struct Inner {
    store: Arc<Mutex<RepositoryStore>>,
    root: PathBuf,
    handlers: Mutex<BTreeMap<String, Arc<dyn JobHandler>>>,
    sink: ProgressSink,
    _session: Lease,
}
struct Lease {
    _file: File,
    _directory: DirectoryGuard,
}
impl Lease {
    fn acquire(root: &Path, name: &str) -> Result<Self, FusionError> {
        let directory = fs_safety::ensure_directory(root)?;
        let path = root.join(name);
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            fs_safety::check_metadata(&metadata, &path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x00200000);
        }
        let file = options.open(&path).map_err(|_| io())?;
        fs_safety::check_metadata(&file.metadata().map_err(|_| io())?, &path)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| busy())?;
        Ok(Self {
            _file: file,
            _directory: directory,
        })
    }
}

pub(crate) fn io() -> FusionError {
    FusionError::new(ErrorCode::Io, "Creator storage operation failed.")
}
fn invalid() -> FusionError {
    FusionError::new(
        ErrorCode::IntegrityViolation,
        "Creator job metadata is inconsistent.",
    )
}
fn conflict() -> FusionError {
    FusionError::new(
        ErrorCode::SnapshotConflict,
        "Creator job inputs or revision changed.",
    )
}
fn busy() -> FusionError {
    FusionError::new(
        ErrorCode::OperationInProgress,
        "A Creator writer already owns this workspace or session.",
    )
}
fn cancelled() -> FusionError {
    FusionError::new(ErrorCode::Cancelled, "Creator job was cancelled.")
}
fn id() -> String {
    format!("{:032x}", rand::random::<u128>())
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>, FusionError> {
    serde_json::to_vec(value).map_err(|_| invalid())
}
fn key(kind: &CreatorJobKind) -> String {
    match kind {
        CreatorJobKind::Scan => "scan",
        CreatorJobKind::Classify => "classify",
        CreatorJobKind::Build => "build",
        CreatorJobKind::Verify => "verify",
        CreatorJobKind::Export => "export",
        CreatorJobKind::VerifyHttps => "verify_https",
    }
    .into()
}
fn status(value: &CreatorJobStatus) -> &'static str {
    match value {
        CreatorJobStatus::Queued => "queued",
        CreatorJobStatus::Running => "running",
        CreatorJobStatus::Cancelling => "cancelling",
        CreatorJobStatus::Cancelled => "cancelled",
        CreatorJobStatus::Interrupted => "interrupted",
        CreatorJobStatus::Completed => "completed",
        CreatorJobStatus::Failed => "failed",
    }
}
fn active(value: &CreatorJobStatus) -> bool {
    matches!(
        value,
        CreatorJobStatus::Queued | CreatorJobStatus::Running | CreatorJobStatus::Cancelling
    )
}
fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn canonical_hash(value: &impl Serialize) -> Result<String, FusionError> {
    Ok(digest(&encode(
        &serde_json::to_value(value).map_err(|_| invalid())?,
    )?))
}
fn unavailable(command: &str) -> FusionError {
    FusionError::new(
        ErrorCode::ActionUnavailable,
        "No Creator handler is registered for this action.",
    )
    .with_detail("command", command)
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, FusionError> {
    mutex.lock().map_err(|_| io())
}

fn load(conn: &Connection, job_id: &str) -> Result<(CreatorJob, String), FusionError> {
    let row: Option<(Vec<u8>, String, String, String, u64)> = conn.query_row("SELECT state_json,run_token,status,project_id,project_revision FROM creator_jobs WHERE job_id=?1", [job_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_| io())?;
    let (bytes, token, stored_status, project_id, revision) =
        row.ok_or_else(|| FusionError::new(ErrorCode::SnapshotNotFound, "Creator job not found."))?;
    let job: CreatorJob = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if job.progress.job_id != job_id
        || job.progress.project_id != project_id
        || job.progress.project_revision != revision
        || status(&job.progress.status) != stored_status
        || !(1..=MAX_REVISION).contains(&job.progress.sequence)
    {
        return Err(invalid());
    }
    Ok((job, token))
}
fn persist(conn: &Connection, job: &mut CreatorJob, token: &str) -> Result<(), FusionError> {
    job.progress.sequence = job
        .progress
        .sequence
        .checked_add(1)
        .filter(|n| *n <= MAX_REVISION)
        .ok_or_else(invalid)?;
    let count = conn
        .execute(
            "UPDATE creator_jobs SET status=?1,state_json=?2 WHERE job_id=?3 AND run_token=?4",
            params![
                status(&job.progress.status),
                encode(job)?,
                job.progress.job_id,
                token
            ],
        )
        .map_err(|_| io())?;
    if count != 1 {
        return Err(conflict());
    }
    Ok(())
}

impl JobRunner {
    pub(crate) fn new(
        store: Arc<Mutex<RepositoryStore>>,
        root: &Path,
        sink: ProgressSink,
    ) -> Result<Self, FusionError> {
        // OS lock precedes recovery; opening another UI/root cannot interrupt a live worker.
        let session = Lease::acquire(root, "creator-session.lock")?;
        let runner = Self(Arc::new(Inner {
            store,
            root: root.to_path_buf(),
            handlers: Mutex::new(BTreeMap::new()),
            sink,
            _session: session,
        }));
        {
            let store = lock(&runner.0.store)?;
            super::jobs_schema::migrate(&store.conn).map_err(|_| io())?;
            let tx =
                rusqlite::Transaction::new_unchecked(&store.conn, TransactionBehavior::Immediate)
                    .map_err(|_| io())?;
            let ids: Vec<String> = tx.prepare("SELECT job_id FROM creator_jobs WHERE status IN ('queued','running','cancelling')").map_err(|_| io())?.query_map([], |r| r.get(0)).map_err(|_| io())?.collect::<rusqlite::Result<_>>().map_err(|_| io())?;
            let mut recovered = Vec::new();
            for id in ids {
                let (mut job, token) = load(&tx, &id)?;
                job.progress.status = CreatorJobStatus::Interrupted;
                job.progress.phase = "interrupted".into();
                persist(&tx, &mut job, &token)?;
                recovered.push(job.progress);
            }
            tx.commit().map_err(|_| io())?;
            for event in recovered {
                (runner.0.sink)(event);
            }
        }
        Ok(runner)
    }
    pub(crate) fn store(&self) -> Arc<Mutex<RepositoryStore>> {
        self.0.store.clone()
    }
    pub fn register(
        &self,
        kind: CreatorJobKind,
        handler: Arc<dyn JobHandler>,
    ) -> Result<(), FusionError> {
        let mut handlers = lock(&self.0.handlers)?;
        if handlers.contains_key(&key(&kind)) {
            return Err(FusionError::new(
                ErrorCode::DuplicateId,
                "Creator handler already registered.",
            ));
        }
        handlers.insert(key(&kind), handler);
        Ok(())
    }
    fn handler(
        &self,
        kind: &CreatorJobKind,
        command: &str,
    ) -> Result<Arc<dyn JobHandler>, FusionError> {
        lock(&self.0.handlers)?
            .get(&key(kind))
            .cloned()
            .ok_or_else(|| unavailable(command))
    }
    pub fn get(&self, job_id: &str) -> Result<CreatorJob, FusionError> {
        Ok(load(&lock(&self.0.store)?.conn, job_id)?.0)
    }
    fn inputs(
        &self,
        project_id: &str,
    ) -> Result<(CreatorProgramProject, Option<SavedPrivateBindings>), FusionError> {
        let store = lock(&self.0.store)?;
        let project = project_storage::open_project(&store, project_id)?.ok_or_else(|| {
            FusionError::new(ErrorCode::SnapshotNotFound, "Creator project not found.")
        })?;
        let bindings = project_storage::open_private_bindings(&store, project_id)?;
        Ok((project, bindings))
    }
    fn stamp(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        handler: &dyn JobHandler,
    ) -> Result<CreatorResumeStamp, FusionError> {
        let mut hashes = handler.input_hashes(project, bindings)?;
        if !hashes.contains_key("executor_implementation")
            || hashes.values().any(|value| !valid_hash(value))
        {
            return Err(invalid());
        }
        for component in &project.program_descriptor.components {
            if !hashes.contains_key(&format!("component:{}", component.id)) {
                return Err(invalid());
            }
        }
        for controls in &project.program_descriptor.controls_profiles {
            if matches!(controls, ProgramControlsProfile::EdenParamPackageV1 { .. })
                && !hashes.contains_key("capability:eden_param_package_v1")
            {
                return Err(invalid());
            }
        }
        hashes.insert(
            "program_descriptor".into(),
            canonical_hash(&project.program_descriptor)?,
        );
        hashes.insert("private_bindings".into(), canonical_hash(&bindings)?);
        Ok(CreatorResumeStamp {
            project_revision: project.revision,
            input_hashes: hashes,
            executor: project.program_descriptor.executor.clone(),
        })
    }
    pub fn start(
        &self,
        kind: CreatorJobKind,
        request: CreatorJobStartRequest,
    ) -> Result<CreatorJobReceipt, FusionError> {
        let handler = self.handler(&kind, &format!("fusion_creator_{}", key(&kind)))?;
        let (project, bindings) = self.inputs(&request.project_id)?;
        validate_request(&project, &request)?;
        let stamp = self.stamp(&project, bindings.as_ref(), &*handler)?;
        let workspace = self
            .0
            .root
            .join("workspaces")
            .join(digest(project.project_id.as_bytes()));
        let lease = Lease::acquire(&workspace, "writer.lock")?;
        let token = id();
        let job_id = format!("job-{}", id());
        let job = CreatorJob {
            progress: CreatorJobProgress {
                job_id: job_id.clone(),
                project_id: project.project_id.clone(),
                project_revision: project.revision,
                sequence: 1,
                kind,
                status: CreatorJobStatus::Queued,
                phase: "queued".into(),
                completed_units: 0,
                total_units: None,
                checkpoint_id: None,
                release_id: None,
                error: None,
            },
            checkpoints: vec![],
            report_ids: vec![],
        };
        {
            let store = lock(&self.0.store)?;
            let tx =
                rusqlite::Transaction::new_unchecked(&store.conn, TransactionBehavior::Immediate)
                    .map_err(|_| io())?;
            current(&store, &project, &stamp)?;
            tx.execute("INSERT INTO creator_jobs(job_id,project_id,project_revision,workspace,status,run_token,state_json,request_json,stamp_json) VALUES(?1,?2,?3,?4,'queued',?5,?6,?7,?8)", params![job_id,project.project_id,project.revision,workspace.to_string_lossy(),token,encode(&job)?,encode(&request)?,encode(&stamp)?]).map_err(|_| busy())?;
            tx.commit().map_err(|_| io())?;
            (self.0.sink)(job.progress.clone());
        }
        let receipt = CreatorJobReceipt {
            job_id: job_id.clone(),
            project_id: project.project_id.clone(),
            project_revision: project.revision,
        };
        self.spawn(
            JobContext {
                runner: self.clone(),
                job_id,
                token,
                project,
                request,
                stamp,
                workspace,
                handler,
            },
            lease,
            None,
        );
        Ok(receipt)
    }
    fn spawn(&self, context: JobContext, lease: Lease, resume: Option<ResumePoint>) {
        tauri::async_runtime::spawn_blocking(move || {
            let _lease = lease;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                context.begin()?;
                context.handler.run(context.clone(), resume)?;
                context.check_cancelled()?;
                context.recheck_stamp()
            }))
            .unwrap_or_else(|_| {
                Err(FusionError::new(
                    ErrorCode::Io,
                    "Creator blocking worker panicked.",
                ))
            });
            let _ = context.finish(result);
        });
    }
    pub fn cancel(&self, job_id: &str) -> Result<CreatorJob, FusionError> {
        let store = lock(&self.0.store)?;
        let (mut job, token) = load(&store.conn, job_id)?;
        if matches!(
            job.progress.status,
            CreatorJobStatus::Queued | CreatorJobStatus::Running
        ) {
            job.progress.status = CreatorJobStatus::Cancelling;
            job.progress.phase = "cancelling".into();
            persist(&store.conn, &mut job, &token)?;
            (self.0.sink)(job.progress.clone());
        }
        Ok(job)
    }
}

fn validate_request(
    project: &CreatorProgramProject,
    request: &CreatorJobStartRequest,
) -> Result<(), FusionError> {
    if project.project_id != request.project_id || project.revision != request.project_revision {
        return Err(conflict());
    }
    let descriptor = &project.program_descriptor;
    super::declarative::compiler::validate_descriptor_metadata(descriptor)?;
    if request.build_profile_ids.len() > 32 || request.controls_profile_ids.len() > 16 {
        return Err(invalid());
    }
    for (i, build_id) in request.build_profile_ids.iter().enumerate() {
        if request.build_profile_ids[..i].contains(build_id) {
            return Err(invalid());
        }
        let build = descriptor
            .build_profiles
            .iter()
            .find(|p| &p.id == build_id)
            .ok_or_else(invalid)?;
        if request
            .controls_profile_ids
            .iter()
            .any(|id| !build.controls_profile_ids.contains(id))
        {
            return Err(invalid());
        }
    }
    for (i, controls) in request.controls_profile_ids.iter().enumerate() {
        if request.controls_profile_ids[..i].contains(controls)
            || !descriptor
                .controls_profiles
                .iter()
                .any(|p| p.id() == controls)
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn current(
    store: &RepositoryStore,
    project: &CreatorProgramProject,
    stamp: &CreatorResumeStamp,
) -> Result<(), FusionError> {
    let actual = project_storage::open_project(store, &project.project_id)?.ok_or_else(conflict)?;
    let bindings = project_storage::open_private_bindings(store, &project.project_id)?;
    if actual.revision != project.revision
        || canonical_hash(&actual.program_descriptor)? != stamp.input_hashes["program_descriptor"]
        || canonical_hash(&bindings.as_ref())? != stamp.input_hashes["private_bindings"]
    {
        return Err(conflict());
    }
    Ok(())
}

#[derive(Clone)]
pub struct JobContext {
    runner: JobRunner,
    job_id: String,
    token: String,
    project: CreatorProgramProject,
    request: CreatorJobStartRequest,
    stamp: CreatorResumeStamp,
    workspace: PathBuf,
    handler: Arc<dyn JobHandler>,
}
impl JobContext {
    pub fn project(&self) -> &CreatorProgramProject {
        &self.project
    }
    pub fn request(&self) -> &CreatorJobStartRequest {
        &self.request
    }
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    /// Synchronous Packer/library scaffold, already running on the runner's blocking
    /// worker. G calls the library here and uses this context at cancellation points.
    pub fn blocking_library<T>(
        &self,
        work: impl FnOnce(&Self) -> Result<T, FusionError>,
    ) -> Result<T, FusionError> {
        self.check_cancelled()?;
        let value = work(self)?;
        self.check_cancelled()?;
        Ok(value)
    }
    pub fn check_cancelled(&self) -> Result<(), FusionError> {
        let store = lock(&self.runner.0.store)?;
        let (job, token) = load(&store.conn, &self.job_id)?;
        if token != self.token || job.progress.status == CreatorJobStatus::Cancelling {
            return Err(cancelled());
        }
        if !matches!(
            job.progress.status,
            CreatorJobStatus::Queued | CreatorJobStatus::Running
        ) {
            return Err(conflict());
        }
        current(&store, &self.project, &self.stamp)
    }
    fn recheck_stamp(&self) -> Result<(), FusionError> {
        let (project, bindings) = self.runner.inputs(&self.project.project_id)?;
        if encode(
            &self
                .runner
                .stamp(&project, bindings.as_ref(), &*self.handler)?,
        )? != encode(&self.stamp)?
        {
            return Err(conflict());
        }
        Ok(())
    }
    fn begin(&self) -> Result<(), FusionError> {
        self.check_cancelled()?;
        self.progress("running", 0, None)
    }
    pub fn progress(
        &self,
        phase: &str,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), FusionError> {
        if phase.chars().count() > 256
            || completed > MAX_REVISION
            || total.is_some_and(|t| t > MAX_REVISION || completed > t)
        {
            return Err(invalid());
        }
        let store = lock(&self.runner.0.store)?;
        let (mut job, token) = load(&store.conn, &self.job_id)?;
        if token != self.token
            || !matches!(
                job.progress.status,
                CreatorJobStatus::Queued | CreatorJobStatus::Running
            )
        {
            return Err(conflict());
        }
        current(&store, &self.project, &self.stamp)?;
        job.progress.status = CreatorJobStatus::Running;
        job.progress.phase = phase.into();
        job.progress.completed_units = completed;
        job.progress.total_units = total;
        persist(&store.conn, &mut job, &token)?;
        (self.runner.0.sink)(job.progress);
        Ok(())
    }
    pub fn checkpoint(
        &self,
        output: CheckpointOutput,
    ) -> Result<CreatorCheckpoint, FusionError> {
        self.check_cancelled()?;
        self.recheck_stamp()?;
        if output.evidence.is_empty()
            || output.evidence.len() > 2 * 1024 * 1024
            || (output.boundary == CreatorCheckpointBoundary::ArtifactCommitted
                && output.artifact_id.as_ref().is_none_or(|id| id.is_empty()))
        {
            return Err(invalid());
        }
        let checkpoint = CreatorCheckpoint {
            checkpoint_id: format!("checkpoint-{}", id()),
            job_id: self.job_id.clone(),
            boundary: output.boundary.clone(),
            stamp: self.stamp.clone(),
            result_blake3: digest(&output.evidence),
            artifact_id: output.artifact_id.clone(),
        };
        if !self
            .handler
            .validate_checkpoint(&checkpoint, &output.evidence)?
        {
            return Err(invalid());
        }
        let store = lock(&self.runner.0.store)?;
        let tx = rusqlite::Transaction::new_unchecked(&store.conn, TransactionBehavior::Immediate)
            .map_err(|_| io())?;
        let (mut job, token) = load(&tx, &self.job_id)?;
        if token != self.token
            || job.progress.status != CreatorJobStatus::Running
            || job.checkpoints.len() >= 4096
        {
            return Err(conflict());
        }
        current(&store, &self.project, &self.stamp)?;
        publish(&tx, &mut job, &output, &self.project)?;
        tx.execute("INSERT INTO creator_checkpoint_results(checkpoint_id,job_id,evidence) VALUES(?1,?2,?3)", params![checkpoint.checkpoint_id,self.job_id,output.evidence]).map_err(|_| io())?;
        job.progress.checkpoint_id = Some(checkpoint.checkpoint_id.clone());
        job.checkpoints.push(checkpoint.clone());
        persist(&tx, &mut job, &token)?;
        tx.commit().map_err(|_| io())?;
        (self.runner.0.sink)(job.progress);
        Ok(checkpoint)
    }
    fn finish(&self, result: Result<(), FusionError>) -> Result<(), FusionError> {
        let store = lock(&self.runner.0.store)?;
        let (mut job, token) = load(&store.conn, &self.job_id)?;
        if token != self.token || !active(&job.progress.status) {
            return Ok(());
        }
        if job.progress.status == CreatorJobStatus::Cancelling
            || result
                .as_ref()
                .err()
                .is_some_and(|e| e.code == ErrorCode::Cancelled)
        {
            job.progress.status = CreatorJobStatus::Cancelled;
            job.progress.error = None;
        } else if let Err(error) = result {
            job.progress.status = CreatorJobStatus::Failed;
            job.progress.error = Some(error);
        } else {
            // Recheck under the same DB lock as terminal publication.
            match current(&store, &self.project, &self.stamp) {
                Ok(()) => job.progress.status = CreatorJobStatus::Completed,
                Err(error) => {
                    job.progress.status = CreatorJobStatus::Failed;
                    job.progress.error = Some(error);
                }
            }
        }
        job.progress.phase = status(&job.progress.status).into();
        persist(&store.conn, &mut job, &token)?;
        (self.runner.0.sink)(job.progress);
        Ok(())
    }
}

fn publish(
    conn: &Connection,
    job: &mut CreatorJob,
    output: &CheckpointOutput,
    project: &CreatorProgramProject,
) -> Result<(), FusionError> {
    if (output.boundary == CreatorCheckpointBoundary::ReleaseBuilt) != output.release.is_some()
        || (output.boundary == CreatorCheckpointBoundary::VerificationCompleted)
            != output.report.is_some()
    {
        return Err(invalid());
    }
    if let Some(release) = &output.release {
        if release.release_id.is_empty()
            || release.project_id != project.project_id
            || release.project_revision != project.revision
            || !valid_hash(&release.manifest_blake3)
            || release
                .artifact_hashes
                .values()
                .any(|hash| !valid_hash(hash))
        {
            return Err(invalid());
        }
        conn.execute("INSERT INTO creator_releases(release_id,project_id,project_revision,release_json) VALUES(?1,?2,?3,?4)", params![release.release_id,release.project_id,release.project_revision,encode(release)?]).map_err(|_| io())?;
        job.progress.release_id = Some(release.release_id.clone());
    }
    if let Some(report) = &output.report {
        let bytes: Vec<u8> = conn.query_row("SELECT release_json FROM creator_releases WHERE release_id=?1 AND project_id=?2 AND project_revision=?3", params![report.release_id,project.project_id,project.revision], |r| r.get(0)).map_err(|_| invalid())?;
        let release: CreatorRelease = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let build = project
            .program_descriptor
            .build_profiles
            .iter()
            .find(|p| p.id == report.build_profile_id)
            .ok_or_else(invalid)?;
        if report.report_id.is_empty()
            || report.project_revision != project.revision
            || report.executor != project.program_descriptor.executor
            || report.release_blake3 != release.manifest_blake3
            || !build
                .controls_profile_ids
                .contains(&report.controls_profile_id)
        {
            return Err(invalid());
        }
        conn.execute("INSERT INTO creator_reports(report_id,project_id,project_revision,report_json) VALUES(?1,?2,?3,?4)", params![report.report_id,project.project_id,project.revision,encode(report)?]).map_err(|_| io())?;
        job.report_ids.push(report.report_id.clone());
        job.progress.release_id = Some(report.release_id.clone());
    }
    Ok(())
}
