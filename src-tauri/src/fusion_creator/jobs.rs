//! Durable Creator runner. Only explicitly registered, trusted host handlers execute work.
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

#[path = "release_content.rs"]
mod release_content;
use release_content::Receipt as ReleaseReceipt;
pub use release_content::ReleaseContent;

pub(crate) type ProgressSink = Arc<dyn Fn(CreatorJobProgress) + Send + Sync>;

/// G implements this host-only API; never accept implementations from a Source.
/// input_hashes MUST audit actual inputs (not echo expected pins), retaining immutable
/// inputs/read leases for the operation. Include component:<id>, executor_implementation,
/// and capability:<id> implementation digests. Called again before reuse/publication.
pub trait JobHandler: Send + Sync {
    fn validate_request(&self, _request: &CreatorJobStartRequest) -> Result<(), FusionError> {
        Ok(())
    }
    /// Read-only validation before the runner creates or acquires the workspace.
    fn validate_write_roots(
        &self,
        _project: &CreatorProgramProject,
        _bindings: Option<&SavedPrivateBindings>,
        _workspace: &Path,
        _release_cas: &Path,
    ) -> Result<(), FusionError> {
        Ok(())
    }
    fn validate_write_roots_for_request(
        &self,
        _request: &CreatorJobStartRequest,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        workspace: &Path,
        release_cas: &Path,
    ) -> Result<(), FusionError> {
        self.validate_write_roots(project, bindings, workspace, release_cas)
    }
    fn input_hashes(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError>;
    /// Defer the selected release and input audit until a worker owns the job.
    fn deferred_input_audit(&self) -> bool {
        false
    }
    fn input_hashes_checked(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        release: Option<&ReleaseContent>,
        check: &dyn Fn() -> Result<(), FusionError>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        check()?;
        let hashes = self.input_hashes_for_release(project, bindings, release)?;
        check()?;
        Ok(hashes)
    }
    fn input_hashes_for_release(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        _release: Option<&ReleaseContent>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        self.input_hashes(project, bindings)
    }
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

/// Resolve existing aliases without creating missing workspace or CAS directories.
pub(super) fn prospective_root(path: &Path) -> std::io::Result<PathBuf> {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => {
                let mut canonical = existing.canonicalize()?;
                canonical.extend(missing.into_iter().rev());
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some((name, parent)) = existing.file_name().zip(existing.parent()) else {
                    return Err(error);
                };
                missing.push(name);
                existing = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn path_contains(parent: &Path, child: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut child = child.components();
        // Canonicalization resolves existing names. Also fold ASCII case in
        // the as-yet-uncreated, host-generated root segments.
        parent.components().all(|part| {
            child
                .next()
                .is_some_and(|next| part.as_os_str().eq_ignore_ascii_case(next.as_os_str()))
        })
    }
    #[cfg(not(windows))]
    child.starts_with(parent)
}

pub(super) fn validate_build_write_roots<'a>(
    components: impl Iterator<Item = (&'a str, &'a Path)>,
    workspace: &Path,
    release_cas: &Path,
) -> Result<(), FusionError> {
    let workspace = prospective_root(workspace);
    let release_cas = prospective_root(release_cas);
    let temp = std::env::temp_dir();
    // Packer resolves parent segments lexically. Don't compare a different
    // tree if the environment supplies a temp path containing them.
    let packer_temp = (!temp
        .components()
        .any(|part| part == std::path::Component::ParentDir))
    .then(|| temp.canonicalize().ok())
    .flatten();
    for (id, input) in components {
        let unresolved = || {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Creator Build could not resolve its input and write directories.",
            )
            .with_detail("componentId", id)
        };
        let input = input.canonicalize().map_err(|_| unresolved())?;
        let workspace = workspace.as_ref().map_err(|_| unresolved())?;
        let release_cas = release_cas.as_ref().map_err(|_| unresolved())?;
        let temp = packer_temp.as_ref().ok_or_else(unresolved)?;
        let overlaps = |root: &Path| path_contains(&input, root) || path_contains(root, &input);
        // Packer creates an exclusive child of the temp directory, never
        // adopts an existing child. Ordinary inputs beneath temp are safe.
        if overlaps(workspace) || overlaps(release_cas) || path_contains(&input, temp) {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "The managed component directory overlaps a Creator Build write location.",
            )
            .with_detail("componentId", id));
        }
    }
    Ok(())
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
    /// Resolve a receipt-backed exact project release and audit actual CAS bytes.
    pub fn read_release_content(
        &self,
        project_id: &str,
        revision: u64,
        release_id: &str,
    ) -> Result<ReleaseContent, FusionError> {
        self.read_release_content_checked(project_id, revision, release_id, &|| Ok(()))
    }

    fn read_release_content_checked(
        &self,
        project_id: &str,
        revision: u64,
        release_id: &str,
        check: &dyn Fn() -> Result<(), FusionError>,
    ) -> Result<ReleaseContent, FusionError> {
        check()?;
        let (release, evidence) = {
            let store = lock(&self.0.store)?;
            let bytes: Vec<u8> = store.conn.query_row(
                "SELECT release_json FROM creator_releases WHERE release_id=?1 AND project_id=?2 AND project_revision=?3",
                params![release_id, project_id, revision], |row| row.get(0),
            ).optional().map_err(|_| io())?.ok_or_else(|| FusionError::new(ErrorCode::SnapshotNotFound, "Creator release not found for this project revision."))?;
            let release: CreatorRelease = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if release.release_id != release_id
                || release.project_id != project_id
                || release.project_revision != revision
            {
                return Err(invalid());
            }
            let checkpoint_id = release_content::checkpoint_id(release_id);
            let (evidence, job_id): (Vec<u8>, String) = store.conn.query_row(
                "SELECT evidence, job_id FROM creator_checkpoint_results WHERE checkpoint_id=?1",
                [&checkpoint_id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(|_| io())?.ok_or_else(release_content::integrity)?;
            let (job, _) = load(&store.conn, &job_id)?;
            if job.progress.project_id != project_id
                || job.progress.project_revision != revision
                || !job.checkpoints.iter().any(|point| {
                    point.checkpoint_id == checkpoint_id
                        && point.job_id == job_id
                        && point.boundary == CreatorCheckpointBoundary::ReleaseBuilt
                        && point.result_blake3 == digest(&evidence)
                })
            {
                return Err(release_content::integrity());
            }
            (release, evidence)
        };
        ReleaseReceipt::audit(&self.0.root, release, &evidence, check)
    }
    fn selected_release(
        &self,
        request: &CreatorJobStartRequest,
        check: &dyn Fn() -> Result<(), FusionError>,
    ) -> Result<Option<ReleaseContent>, FusionError> {
        request
            .release_id
            .as_deref()
            .map(|id| {
                self.read_release_content_checked(
                    &request.project_id,
                    request.project_revision,
                    id,
                    check,
                )
            })
            .transpose()
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
        release: Option<&ReleaseContent>,
    ) -> Result<CreatorResumeStamp, FusionError> {
        self.stamp_checked(project, bindings, handler, release, &|| Ok(()))
    }
    fn stamp_checked(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        handler: &dyn JobHandler,
        release: Option<&ReleaseContent>,
        check: &dyn Fn() -> Result<(), FusionError>,
    ) -> Result<CreatorResumeStamp, FusionError> {
        let mut hashes = handler.input_hashes_checked(project, bindings, release, check)?;
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
        hashes.remove("selected_release");
        if let Some(release) = release {
            hashes.insert("selected_release".into(), release.receipt_blake3().into());
        }
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
        handler.validate_request(&request)?;
        let (project, bindings) = self.inputs(&request.project_id)?;
        validate_request(&project, &request)?;
        let workspace = self.workspace(&project.project_id);
        handler.validate_write_roots_for_request(
            &request,
            &project,
            bindings.as_ref(),
            &workspace,
            &self.release_cas(),
        )?;
        let lease = Lease::acquire(&workspace, "writer.lock")?;
        let deferred = handler.deferred_input_audit();
        let release = if deferred {
            None
        } else {
            self.selected_release(&request, &|| Ok(()))?
        };
        let stamp = if deferred {
            CreatorResumeStamp {
                project_revision: project.revision,
                executor: project.program_descriptor.executor.clone(),
                input_hashes: BTreeMap::from([
                    (
                        "program_descriptor".into(),
                        canonical_hash(&project.program_descriptor)?,
                    ),
                    (
                        "private_bindings".into(),
                        canonical_hash(&bindings.as_ref())?,
                    ),
                ]),
            }
        } else {
            self.stamp(&project, bindings.as_ref(), &*handler, release.as_ref())?
        };
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
                release_id: request.release_id.clone(),
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
                release,
                workspace,
                handler,
            },
            lease,
            None,
            deferred,
        );
        Ok(receipt)
    }
    fn spawn(
        &self,
        mut context: JobContext,
        lease: Lease,
        resume: Option<ResumePoint>,
        deferred: bool,
    ) {
        tauri::async_runtime::spawn_blocking(move || {
            let _lease = lease;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                context.begin()?;
                if deferred {
                    if resume.is_some() {
                        context.verify_resumed_stamp()?;
                    } else {
                        context.initialize_stamp()?;
                    }
                }
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
    fn workspace(&self, project_id: &str) -> PathBuf {
        self.0
            .root
            .join("workspaces")
            .join(digest(project_id.as_bytes()))
    }

    fn release_cas(&self) -> PathBuf {
        // Receipt capture opens CasStore with the runner root. Opening the
        // store here would itself create directories before preflight.
        self.0.root.join(".fusion-source").join("cas")
    }

    /// Resume only a recovered run with durable evidence. Audit the entire reused prefix:
    /// a valid latest result does not make a missing earlier artifact safe to reuse.
    pub fn resume(&self, job_id: &str) -> Result<CreatorJobReceipt, FusionError> {
        let (previous, previous_token) = {
            let store = lock(&self.0.store)?;
            load(&store.conn, job_id)?
        };
        let handler = self.handler(&previous.progress.kind, "fusion_creator_job_resume")?;
        if previous.progress.status != CreatorJobStatus::Interrupted {
            return Err(conflict());
        }
        let workspace = self.workspace(&previous.progress.project_id);
        let (request_bytes, stamp_bytes, saved_workspace): (Vec<u8>, Vec<u8>, String) = {
            let store = lock(&self.0.store)?;
            store.conn.query_row(
                "SELECT request_json,stamp_json,workspace FROM creator_jobs WHERE job_id=?1 AND run_token=?2",
                params![job_id, previous_token],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).map_err(|_| conflict())?
        };
        let request: CreatorJobStartRequest =
            serde_json::from_slice(&request_bytes).map_err(|_| invalid())?;
        handler.validate_request(&request)?;
        let saved_stamp: CreatorResumeStamp =
            serde_json::from_slice(&stamp_bytes).map_err(|_| invalid())?;
        if request.project_id != previous.progress.project_id
            || request.project_revision != previous.progress.project_revision
            || saved_stamp.project_revision != previous.progress.project_revision
            || saved_workspace != workspace.to_string_lossy()
            || previous.checkpoints.len() > 4096
        {
            return Err(invalid());
        }
        let last = previous.checkpoints.last().ok_or_else(conflict)?;
        if previous.progress.checkpoint_id.as_deref() != Some(last.checkpoint_id.as_str()) {
            return Err(invalid());
        }
        let (project, bindings) = self.inputs(&request.project_id)?;
        validate_request(&project, &request)?;
        handler.validate_write_roots_for_request(
            &request,
            &project,
            bindings.as_ref(),
            &workspace,
            &self.release_cas(),
        )?;
        let lease = Lease::acquire(&workspace, "writer.lock")?;
        let deferred = handler.deferred_input_audit();
        let (release, stamp) = if deferred {
            if saved_stamp.project_revision != project.revision
                || saved_stamp.executor != project.program_descriptor.executor
                || !saved_stamp
                    .input_hashes
                    .contains_key("executor_implementation")
                || !saved_stamp.input_hashes.contains_key("program_descriptor")
                || !saved_stamp.input_hashes.contains_key("private_bindings")
                || (request.release_id.is_some()
                    && !saved_stamp.input_hashes.contains_key("selected_release"))
                || project
                    .program_descriptor
                    .components
                    .iter()
                    .any(|component| {
                        !saved_stamp
                            .input_hashes
                            .contains_key(&format!("component:{}", component.id))
                    })
                || project
                    .program_descriptor
                    .controls_profiles
                    .iter()
                    .any(|controls| {
                        matches!(controls, ProgramControlsProfile::EdenParamPackageV1 { .. })
                            && !saved_stamp
                                .input_hashes
                                .contains_key("capability:eden_param_package_v1")
                    })
                || saved_stamp
                    .input_hashes
                    .values()
                    .any(|value| !valid_hash(value))
            {
                return Err(conflict());
            }
            {
                let store = lock(&self.0.store)?;
                current(&store, &project, &saved_stamp)?;
            }
            (None, saved_stamp)
        } else {
            let release = self.selected_release(&request, &|| Ok(()))?;
            let stamp = self.stamp(&project, bindings.as_ref(), &*handler, release.as_ref())?;
            if encode(&saved_stamp)? != encode(&stamp)? {
                return Err(conflict());
            }
            (release, stamp)
        };
        let encoded_stamp = encode(&stamp)?;

        let mut resume = None;
        let mut checkpoint_ids = std::collections::BTreeSet::new();
        for checkpoint in &previous.checkpoints {
            if checkpoint.job_id != job_id
                || !checkpoint_ids.insert(&checkpoint.checkpoint_id)
                || encode(&checkpoint.stamp)? != encoded_stamp
                || (checkpoint.boundary == CreatorCheckpointBoundary::ArtifactCommitted
                    && checkpoint.artifact_id.as_ref().is_none_or(String::is_empty))
            {
                return Err(invalid());
            }
            let evidence: Vec<u8> = {
                let store = lock(&self.0.store)?;
                store.conn.query_row(
                    "SELECT evidence FROM creator_checkpoint_results WHERE job_id=?1 AND checkpoint_id=?2",
                    params![job_id, checkpoint.checkpoint_id],
                    |row| row.get(0),
                ).optional().map_err(|_| io())?.ok_or_else(invalid)?
            };
            if evidence.is_empty()
                || evidence.len() > 2 * 1024 * 1024
                || digest(&evidence) != checkpoint.result_blake3
            {
                return Err(invalid());
            }
            if !deferred
                && checkpoint.boundary == CreatorCheckpointBoundary::ReleaseBuilt
                && checkpoint
                    .checkpoint_id
                    .starts_with(release_content::CHECKPOINT_PREFIX)
            {
                let receipt: ReleaseReceipt =
                    serde_json::from_slice(&evidence).map_err(|_| invalid())?;
                if checkpoint.checkpoint_id != release_content::checkpoint_id(&receipt.release_id) {
                    return Err(invalid());
                }
                self.read_release_content(
                    &project.project_id,
                    project.revision,
                    &receipt.release_id,
                )?;
            } else if ((deferred || !cfg!(test))
                && checkpoint.boundary == CreatorCheckpointBoundary::ReleaseBuilt)
                || !handler.validate_checkpoint(checkpoint, &evidence)?
            {
                return Err(invalid());
            }
            resume = Some(ResumePoint {
                checkpoint: checkpoint.clone(),
                evidence,
            });
        }

        let token = id();
        {
            let store = lock(&self.0.store)?;
            let tx =
                rusqlite::Transaction::new_unchecked(&store.conn, TransactionBehavior::Immediate)
                    .map_err(|_| io())?;
            let (mut job, db_token) = load(&tx, job_id)?;
            if db_token != previous_token || job.progress.sequence != previous.progress.sequence {
                return Err(conflict());
            }
            current(&store, &project, &stamp)?;
            let count = tx.execute(
                "UPDATE creator_jobs SET run_token=?1 WHERE job_id=?2 AND run_token=?3 AND status='interrupted'",
                params![token, job_id, previous_token],
            ).map_err(|_| io())?;
            if count != 1 {
                return Err(conflict());
            }
            job.progress.status = CreatorJobStatus::Queued;
            job.progress.phase = "queued".into();
            job.progress.error = None;
            persist(&tx, &mut job, &token)?;
            tx.commit().map_err(|_| io())?;
            (self.0.sink)(job.progress);
        }
        let receipt = CreatorJobReceipt {
            job_id: job_id.into(),
            project_id: project.project_id.clone(),
            project_revision: project.revision,
        };
        self.spawn(
            JobContext {
                runner: self.clone(),
                job_id: job_id.into(),
                token,
                project,
                request,
                stamp,
                release,
                workspace,
                handler,
            },
            lease,
            resume,
            deferred,
        );
        Ok(receipt)
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
    release: Option<ReleaseContent>,
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
    pub fn selected_release(&self) -> Option<&ReleaseContent> {
        self.release.as_ref()
    }
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    pub(super) fn private_bindings(&self) -> Result<Option<SavedPrivateBindings>, FusionError> {
        let store = lock(&self.runner.0.store)?;
        current(&store, &self.project, &self.stamp)?;
        project_storage::open_private_bindings(&store, &self.project.project_id)
    }
    pub(super) fn matches_input(&self, key: &str, digest: &str) -> bool {
        self.stamp
            .input_hashes
            .get(key)
            .is_some_and(|expected| expected == digest)
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
    /// Poll durable state in chunked I/O without locking SQLite on every read.
    pub(super) fn with_work_checks<T>(
        &self,
        work: impl FnOnce(&dyn Fn() -> Result<(), FusionError>) -> Result<T, FusionError>,
    ) -> Result<T, FusionError> {
        self.check_cancelled()?;
        let last = std::cell::Cell::new(std::time::Instant::now());
        let result = work(&|| {
            if last.get().elapsed() >= std::time::Duration::from_millis(100) {
                self.check_cancelled()?;
                last.set(std::time::Instant::now());
            }
            Ok(())
        })?;
        self.check_cancelled()?;
        Ok(result)
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
    fn initialize_stamp(&mut self) -> Result<(), FusionError> {
        self.progress("hashing_inputs", 0, None)?;
        let bindings = self.private_bindings()?;
        self.release =
            self.with_work_checks(|check| self.runner.selected_release(&self.request, check))?;
        let stamp = self.runner.stamp_checked(
            &self.project,
            bindings.as_ref(),
            &*self.handler,
            self.release.as_ref(),
            &|| self.check_cancelled(),
        )?;
        self.check_cancelled()?;
        {
            let store = lock(&self.runner.0.store)?;
            current(&store, &self.project, &self.stamp)?;
            let updated = store.conn.execute(
                "UPDATE creator_jobs SET stamp_json=?1 WHERE job_id=?2 AND run_token=?3 AND status='running'",
                params![encode(&stamp)?, self.job_id, self.token],
            ).map_err(|_| io())?;
            if updated != 1 {
                return Err(cancelled());
            }
        }
        self.stamp = stamp;
        Ok(())
    }
    fn verify_resumed_stamp(&mut self) -> Result<(), FusionError> {
        self.progress("auditing_resume", 0, None)?;
        let bindings = self.private_bindings()?;
        self.release =
            self.with_work_checks(|check| self.runner.selected_release(&self.request, check))?;
        let actual = self.runner.stamp_checked(
            &self.project,
            bindings.as_ref(),
            &*self.handler,
            self.release.as_ref(),
            &|| self.check_cancelled(),
        )?;
        if encode(&actual)? != encode(&self.stamp)? {
            return Err(conflict());
        }
        self.check_cancelled()
    }
    fn recheck_stamp(&self) -> Result<(), FusionError> {
        let (project, bindings) = self.runner.inputs(&self.project.project_id)?;
        let release =
            self.with_work_checks(|check| self.runner.selected_release(&self.request, check))?;
        if encode(&self.runner.stamp_checked(
            &project,
            bindings.as_ref(),
            &*self.handler,
            release.as_ref(),
            &|| self.check_cancelled(),
        )?)? != encode(&self.stamp)?
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
    pub fn checkpoint(&self, output: CheckpointOutput) -> Result<CreatorCheckpoint, FusionError> {
        // Legacy synthetic recovery fixtures predate receipts and remain test-only.
        if output.boundary == CreatorCheckpointBoundary::ReleaseBuilt && !cfg!(test) {
            return Err(release_content::integrity());
        }
        self.commit_checkpoint(output, None, false)
    }

    pub(super) fn release_reports(
        &self,
        release_id: &str,
    ) -> Result<Vec<CreatorReport>, FusionError> {
        let store = lock(&self.runner.0.store)?;
        let mut stmt = store
            .conn
            .prepare(
                "SELECT report_json FROM creator_reports WHERE project_id=?1 AND project_revision=?2",
            )
            .map_err(|_| io())?;
        let mut rows = stmt
            .query(params![self.project.project_id, self.project.revision])
            .map_err(|_| io())?;
        let mut reports = Vec::new();
        while let Some(row) = rows.next().map_err(|_| io())? {
            let bytes: Vec<u8> = row.get(0).map_err(|_| invalid())?;
            let report: CreatorReport = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if report.release_id == release_id {
                reports.push(report);
            }
        }
        Ok(reports)
    }

    pub(super) fn checkpoint_verified(
        &self,
        output: CheckpointOutput,
    ) -> Result<CreatorCheckpoint, FusionError> {
        let expected_kind = match self.progress_kind_expected() {
            CreatorJobKind::Verify => CreatorReportKind::CleanInstall,
            CreatorJobKind::VerifyHttps => CreatorReportKind::HttpsAcquisition,
            _ => return Err(invalid()),
        };
        if output.boundary != CreatorCheckpointBoundary::VerificationCompleted
            || !output.report.as_ref().is_some_and(|report| {
                report.kind == expected_kind && report.status == CreatorReportStatus::Passed
            })
        {
            return Err(invalid());
        }
        self.commit_checkpoint(output, None, true)
    }

    pub(super) fn checkpoint_exported(
        &self,
        evidence: Vec<u8>,
    ) -> Result<CreatorCheckpoint, FusionError> {
        if self.progress_kind_expected() != CreatorJobKind::Export {
            return Err(invalid());
        }
        self.commit_checkpoint(
            CheckpointOutput {
                boundary: CreatorCheckpointBoundary::ExportCompleted,
                evidence,
                artifact_id: None,
                release: None,
                report: None,
            },
            None,
            true,
        )
    }

    fn progress_kind_expected(&self) -> CreatorJobKind {
        if self.request.source_url.is_some() {
            CreatorJobKind::VerifyHttps
        } else if self.request.target_directory.is_some() {
            CreatorJobKind::Export
        } else if self.request.release_id.is_some() {
            CreatorJobKind::Verify
        } else {
            CreatorJobKind::Build
        }
    }

    pub(super) fn fail_with_report(
        &self,
        report: CreatorReport,
        error: FusionError,
    ) -> Result<(), FusionError> {
        self.check_cancelled()?;
        self.recheck_stamp()?;
        let expected = match report.kind {
            CreatorReportKind::CleanInstall => CreatorJobKind::Verify,
            CreatorReportKind::HttpsAcquisition => CreatorJobKind::VerifyHttps,
            _ => return Err(invalid()),
        };
        if report.status != CreatorReportStatus::Failed
            || self
                .request
                .release_id
                .as_ref()
                .is_some_and(|id| *id != report.release_id)
        {
            return Err(invalid());
        }
        let store = lock(&self.runner.0.store)?;
        let tx = rusqlite::Transaction::new_unchecked(&store.conn, TransactionBehavior::Immediate)
            .map_err(|_| io())?;
        let (mut job, token) = load(&tx, &self.job_id)?;
        if token != self.token
            || job.progress.status != CreatorJobStatus::Running
            || job.progress.kind != expected
        {
            return Err(conflict());
        }
        current(&store, &self.project, &self.stamp)?;
        publish_report_row(&tx, &mut job, &report, &self.project)?;
        job.progress.status = CreatorJobStatus::Failed;
        job.progress.phase = "failed".into();
        job.progress.error = Some(error);
        persist(&tx, &mut job, &token)?;
        tx.commit().map_err(|_| io())?;
        (self.runner.0.sink)(job.progress);
        Ok(())
    }

    /// Commit manifest.json and artifacts/<id> from a direct child of the owned workspace.
    pub fn checkpoint_release(
        &self,
        release: CreatorRelease,
        staging_directory: &str,
    ) -> Result<CreatorCheckpoint, FusionError> {
        self.check_cancelled()?;
        if self.release.is_some()
            || release.project_id != self.project.project_id
            || release.project_revision != self.project.revision
        {
            return Err(conflict());
        }
        let stage = fs_safety::validate_relative_path(staging_directory)?;
        if stage.components().count() != 1 {
            return Err(release_content::integrity());
        }
        let evidence = self.with_work_checks(|check| {
            ReleaseReceipt::capture(
                &self.runner.0.root,
                &self.workspace.join(stage),
                &release,
                check,
            )
        })?;
        let checkpoint_id = release_content::checkpoint_id(&release.release_id);
        self.commit_checkpoint(
            CheckpointOutput {
                boundary: CreatorCheckpointBoundary::ReleaseBuilt,
                evidence,
                artifact_id: None,
                release: Some(release),
                report: None,
            },
            Some(checkpoint_id),
            false,
        )
    }

    fn commit_checkpoint(
        &self,
        output: CheckpointOutput,
        receipt_id: Option<String>,
        final_verification: bool,
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
        if output.report.as_ref().is_some_and(|report| {
            self.request
                .release_id
                .as_ref()
                .is_some_and(|id| *id != report.release_id)
        }) {
            return Err(conflict());
        }
        let receipt_backed = receipt_id.is_some();
        let checkpoint = CreatorCheckpoint {
            checkpoint_id: receipt_id.unwrap_or_else(|| format!("checkpoint-{}", id())),
            job_id: self.job_id.clone(),
            boundary: output.boundary.clone(),
            stamp: self.stamp.clone(),
            result_blake3: digest(&output.evidence),
            artifact_id: output.artifact_id.clone(),
        };
        if !receipt_backed
            && !self
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
            || (final_verification
                && !matches!(
                    job.progress.kind,
                    CreatorJobKind::Verify | CreatorJobKind::VerifyHttps | CreatorJobKind::Export
                ))
        {
            return Err(conflict());
        }
        current(&store, &self.project, &self.stamp)?;
        publish(&tx, &mut job, &output, &self.project)?;
        tx.execute("INSERT INTO creator_checkpoint_results(checkpoint_id,job_id,evidence) VALUES(?1,?2,?3)", params![checkpoint.checkpoint_id,self.job_id,output.evidence]).map_err(|_| io())?;
        job.progress.checkpoint_id = Some(checkpoint.checkpoint_id.clone());
        job.checkpoints.push(checkpoint.clone());
        // Final Build/Verify evidence and Completed share the same transaction.
        if (receipt_backed && job.progress.kind == CreatorJobKind::Build) || final_verification {
            job.progress.status = CreatorJobStatus::Completed;
            job.progress.phase = "completed".into();
        }
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
        publish_report_row(conn, job, report, project)?;
    }
    Ok(())
}

fn publish_report_row(
    conn: &Connection,
    job: &mut CreatorJob,
    report: &CreatorReport,
    project: &CreatorProgramProject,
) -> Result<(), FusionError> {
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
    Ok(())
}
