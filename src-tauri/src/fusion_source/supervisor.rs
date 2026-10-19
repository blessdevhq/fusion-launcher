//! Direct process execution with a group-owned workspace lease. The detached
//! ownership worker outlives cancelled futures and reconciles saves only after
//! the Windows Job Object is empty. Raw log tails are internal Rust data; only
//! `sanitized_crash_report` is suitable for future IPC or persistence.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::oneshot;

use super::errors::{ErrorCode, FusionError};
use super::evolution::SchemaUpcaster;
use super::integrity::{
    ExecutableTrust, ManifestSyncVerifier, PreLaunchIntegrityGuard, VerifiedCriticalFiles,
};
use super::schema::{ProcessRole, ProcessSpec};
use super::telemetry::{
    CrashOutcome, CrashReportBuilder, DeduplicationDecision, DeduplicationLedger,
    DeduplicationPolicy, SanitizedCrashReport, TelemetrySanitizer,
};
use super::workspace::{AssembledWorkspace, WorkspaceSnapshot};

#[cfg(windows)]
#[path = "supervisor_windows.rs"]
mod platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// Intentionally not serializable: sanitize before sending logs outside the core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub process_id: String,
    pub stream: LogStream,
    pub text: String,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LogLimits {
    pub max_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
}
impl Default for LogLimits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024,
            max_lines: 1024,
            max_line_bytes: 8 * 1024,
        }
    }
}
impl LogLimits {
    fn validate(self) -> Result<Self, FusionError> {
        if self.max_bytes == 0
            || self.max_lines == 0
            || self.max_line_bytes == 0
            || self.max_bytes > 64 * 1024 * 1024
            || self.max_lines > 100_000
            || self.max_line_bytes > self.max_bytes
            || self.max_line_bytes > 64 * 1024
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Invalid process log limits.",
            ));
        }
        Ok(self)
    }
}

#[derive(Debug)]
pub struct BoundedLogRingBuffer {
    limits: LogLimits,
    lines: VecDeque<LogLine>,
    bytes: usize,
    dropped_lines: u64,
}
impl BoundedLogRingBuffer {
    pub fn new(limits: LogLimits) -> Result<Self, FusionError> {
        Ok(Self {
            limits: limits.validate()?,
            lines: VecDeque::new(),
            bytes: 0,
            dropped_lines: 0,
        })
    }
    pub fn push(&mut self, mut line: LogLine) {
        // IDs have their own canonical bound; still charge them to the ring.
        let available = self.limits.max_bytes.saturating_sub(line.process_id.len());
        if available == 0 {
            self.dropped_lines = self.dropped_lines.saturating_add(1);
            return;
        }
        let limit = self.limits.max_line_bytes.min(available);
        if line.text.len() > limit {
            truncate_utf8(&mut line.text, limit);
            line.truncated = true;
        }
        let size = line.text.len() + line.process_id.len();
        while self.lines.len() >= self.limits.max_lines || self.bytes + size > self.limits.max_bytes
        {
            if let Some(old) = self.lines.pop_front() {
                self.bytes -= old.text.len() + old.process_id.len();
                self.dropped_lines = self.dropped_lines.saturating_add(1);
            } else {
                break;
            }
        }
        self.bytes += size;
        self.lines.push_back(line);
    }
    pub fn lines(&self) -> Vec<LogLine> {
        self.lines.iter().cloned().collect()
    }
    pub fn byte_len(&self) -> usize {
        self.bytes
    }
    pub fn dropped_lines(&self) -> u64 {
        self.dropped_lines
    }
}

fn truncate_utf8(value: &mut String, mut limit: usize) {
    while !value.is_char_boundary(limit) {
        limit -= 1;
    }
    value.truncate(limit);
}
fn lock_logs(logs: &Mutex<BoundedLogRingBuffer>) -> MutexGuard<'_, BoundedLogRingBuffer> {
    logs.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Debug, Default)]
pub struct SupervisorOptions {
    pub logs: LogLimits,
    pub deduplication_policy: DeduplicationPolicy,
    /// Optional additional redaction runs before the mandatory core sanitizer.
    /// It can never replace OS/workspace/local-base/environment redaction.
    pub telemetry: Option<TelemetrySanitizer>,
}

/// This outcome remains inside the core. Only the sanitized report is an IPC DTO.
#[derive(Debug)]
pub struct ProcessOutcome {
    pub primary_exit_code: i32,
    pub user_cancelled: bool,
    pub process_exit_codes: BTreeMap<String, i32>,
    pub log_tail: Vec<LogLine>,
    pub dropped_log_lines: u64,
    pub sanitized_crash_report: Option<SanitizedCrashReport>,
    pub crash_deduplication: Option<DeduplicationDecision>,
}

pub struct RunningProcessGroup {
    cancel: Arc<AtomicBool>,
    logs: Arc<Mutex<BoundedLogRingBuffer>>,
    completion: Option<oneshot::Receiver<Result<ProcessOutcome, FusionError>>>,
    finished: bool,
}

#[derive(Clone)]
pub struct ProcessCancellation {
    cancel: Arc<AtomicBool>,
}
impl ProcessCancellation {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl RunningProcessGroup {
    pub fn cancellation_handle(&self) -> ProcessCancellation {
        ProcessCancellation {
            cancel: Arc::clone(&self.cancel),
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    /// A bounded diagnostic preview for internal consumers. Future IPC adapters
    /// must run the telemetry sanitizer before exposing any of these strings.
    pub fn log_tail(&self) -> Vec<LogLine> {
        lock_logs(&self.logs).lines()
    }
    pub async fn wait(mut self) -> Result<ProcessOutcome, FusionError> {
        let receiver = self
            .completion
            .take()
            .expect("completion receiver consumed once");
        let result = receiver.await.map_err(|_| {
            FusionError::new(
                ErrorCode::ProcessSupervisionFailed,
                "Process ownership worker ended unexpectedly.",
            )
        })?;
        self.finished = true;
        result
    }
}
impl Drop for RunningProcessGroup {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel();
        }
    }
}

pub struct ProcessSupervisor;
impl ProcessSupervisor {
    /// Takes exclusive ownership of the lease. A cancelled caller cannot release
    /// it while any member or descendant of the group is still writing saves.
    pub async fn launch(
        lease: AssembledWorkspace,
        trust: Arc<dyn ExecutableTrust>,
        sync: Option<Arc<dyn ManifestSyncVerifier>>,
        options: SupervisorOptions,
    ) -> Result<RunningProcessGroup, FusionError> {
        lease.verify_execution_contract()?;
        SchemaUpcaster::validate_canonical(&lease.snapshot.manifest)?;
        options.logs.validate()?;
        if lease
            .snapshot
            .manifest
            .orchestrator
            .as_ref()
            .is_some_and(|spec| spec.strict_sync)
        {
            let digest = lease
                .snapshot
                .source_manifest_blake3
                .as_deref()
                .ok_or_else(|| {
                    FusionError::new(
                        ErrorCode::StrictSyncRequired,
                        "Strict sync requires the verified raw manifest hash.",
                    )
                })?;
            let verifier = sync.ok_or_else(|| {
                FusionError::new(
                    ErrorCode::StrictSyncRequired,
                    "Strict sync requires a fresh source-index verification before launch.",
                )
            })?;
            verifier
                .verify_current(&lease.snapshot.game, digest)
                .await?;
        }
        #[cfg(not(windows))]
        {
            let _ = (lease, trust, options);
            Err(FusionError::new(
                ErrorCode::UnsupportedPlatform,
                "Safe process-tree supervision is currently available on Windows only.",
            ))
        }
        #[cfg(windows)]
        {
            let logs = Arc::new(Mutex::new(BoundedLogRingBuffer::new(options.logs)?));
            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, rx) = oneshot::channel();
            let worker_logs = Arc::clone(&logs);
            let worker_cancel = Arc::clone(&cancel);
            std::thread::Builder::new()
                .name("fusion-process-owner".into())
                .spawn(move || {
                    let result =
                        run_group(lease, trust.as_ref(), &worker_cancel, &worker_logs, options);
                    let _ = tx.send(result);
                })
                .map_err(|error| {
                    FusionError::new(
                        ErrorCode::ProcessLaunchFailed,
                        "Cannot start process ownership worker.",
                    )
                    .with_detail("osError", error.to_string())
                })?;
            Ok(RunningProcessGroup {
                cancel,
                logs,
                completion: Some(rx),
                finished: false,
            })
        }
    }
}

#[cfg(windows)]
struct OwnedGroup {
    job: platform::WindowsJob,
    lease: Option<AssembledWorkspace>,
    executable_locks: Option<VerifiedCriticalFiles>,
    processes: Vec<(String, platform::WindowsProcess)>,
    exit_codes: BTreeMap<String, i32>,
    readers: Vec<std::thread::JoinHandle<()>>,
    finished: bool,
}

#[cfg(windows)]
impl OwnedGroup {
    fn finish(&mut self) -> Result<(), FusionError> {
        if self.finished {
            return Ok(());
        }
        let mut first_error = self.job.terminate().err();
        // Query failures cannot prove writers stopped. Fail closed by retaining
        // the lease and retrying termination; the handle remains owned here.
        loop {
            match self.job.is_empty() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    let _ = self.job.terminate();
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Job accounting can observe zero before the final root-process
        // handle becomes signaled. Observe each known process's actual exit,
        // then close its kernel/image handle before releasing file leases.
        for (id, process) in &self.processes {
            loop {
                match process.try_wait() {
                    Ok(Some(code)) => {
                        self.exit_codes.insert(id.clone(), code);
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(1)),
                    Err(error) => {
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                        break;
                    }
                }
            }
        }
        self.processes.clear();
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        let reconciliation = self.lease.as_ref().expect("owned lease").reconcile_saves();
        // Reconciliation precedes closing every read/ownership descriptor.
        drop(self.executable_locks.take());
        drop(self.lease.take());
        self.finished = true;
        reconciliation?;
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for OwnedGroup {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(windows)]
fn run_group(
    lease: AssembledWorkspace,
    trust: &dyn ExecutableTrust,
    cancel: &AtomicBool,
    logs: &Arc<Mutex<BoundedLogRingBuffer>>,
    options: SupervisorOptions,
) -> Result<ProcessOutcome, FusionError> {
    // All fallible preparation before job creation has no writers to reconcile.
    lease.verify_execution_contract()?;
    let snapshot = lease.snapshot.clone();
    let specs = process_specs(&snapshot)?;
    if specs.len() > 64 {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "A supervised process graph exceeds the local process/log budget.",
        )
        .with_detail("maxProcesses", 64));
    }
    let environment = lease.isolated_environment()?;
    let telemetry_directory = lease.telemetry_directory()?;
    let commands: Vec<_> = specs
        .iter()
        .map(|spec| prepare_command(spec, &snapshot.workspace_root, &environment))
        .collect::<Result<_, _>>()?;
    let job = platform::WindowsJob::new()?;
    let mut group = OwnedGroup {
        job,
        lease: Some(lease),
        executable_locks: None,
        processes: Vec::new(),
        exit_codes: BTreeMap::new(),
        readers: Vec::new(),
        finished: false,
    };
    let operation = (|| {
        // Locks stay with the ownership worker until the ENTIRE job is empty.
        group.executable_locks = Some(PreLaunchIntegrityGuard::verify_and_lock(&snapshot, trust)?);
        for (spec, command) in specs.iter().zip(&commands) {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            let mut process = group.job.spawn(command)?;
            // Publish the process handle before any reader-thread allocation:
            // an allocation/spawn error still goes through OwnedGroup::drop.
            let stdout = process.stdout.take().expect("stdout pipe");
            let stderr = process.stderr.take().expect("stderr pipe");
            group.processes.push((spec.id.clone(), process));
            group.readers.push(reader(
                stdout,
                spec.id.clone(),
                LogStream::Stdout,
                Arc::clone(logs),
                options.logs,
            )?);
            group.readers.push(reader(
                stderr,
                spec.id.clone(),
                LogStream::Stderr,
                Arc::clone(logs),
                options.logs,
            )?);
        }
        let primary = specs
            .iter()
            .find(|spec| spec.role == ProcessRole::Primary)
            .expect("validated primary process");
        let mut exits = BTreeMap::new();
        let user_cancelled;
        loop {
            for (id, process) in &group.processes {
                if !exits.contains_key(id) {
                    if let Some(code) = process.try_wait()? {
                        exits.insert(id.clone(), code);
                    }
                }
            }
            if cancel.load(Ordering::Acquire) {
                user_cancelled = true;
                break;
            }
            if exits.contains_key(&primary.id) {
                user_cancelled = false;
                break;
            }
            if group.job.is_empty()? {
                user_cancelled = false;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok((primary.id.clone(), exits, user_cancelled))
    })();
    // Includes startup/reader errors and user cancellation. Never release the
    // workspace just because a root process has exited while descendants live.
    let finish = group.finish();
    let (primary, mut exits, user_cancelled) = match (operation, finish) {
        (Ok(outcome), Ok(())) => outcome,
        (Err(error), Err(cleanup)) => return Err(error.with_detail("cleanupError", cleanup)),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => return Err(error),
    };
    exits.extend(group.exit_codes.clone());
    let primary_exit_code = exits.get(&primary).copied().unwrap_or(1);
    let ring = lock_logs(logs);
    let log_tail = ring.lines();
    let dropped_log_lines = ring.dropped_lines();
    drop(ring);
    let joined = log_tail
        .iter()
        .map(|line| format!("[{}/{:?}] {}", line.process_id, line.stream, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let sanitizer = TelemetrySanitizer::for_snapshot_environment(&snapshot, &environment);
    // Mandatory redaction precedes custom truncation, so an extra sanitizer's
    // tiny tail cannot remove a path/token prefix before the core recognizes it.
    let mandatory = sanitizer.sanitize_logs(&joined);
    let joined = options.telemetry.as_ref().map_or_else(
        || mandatory.as_str().to_owned(),
        |extra| extra.sanitize_logs(mandatory.as_str()).as_str().to_owned(),
    );
    let mut sanitized_crash_report = CrashReportBuilder::build(
        &snapshot,
        CrashOutcome {
            primary_exit_code,
            user_cancelled,
        },
        &joined,
        &sanitizer,
    )?;
    let crash_deduplication = if let Some(report) = &sanitized_crash_report {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| {
                FusionError::new(
                    ErrorCode::ProcessSupervisionFailed,
                    "System clock predates the Unix epoch.",
                )
            })?
            .as_secs();
        let decision =
            DeduplicationLedger::open(&telemetry_directory, options.deduplication_policy)?
                .admit(report, now)?;
        if !matches!(decision, DeduplicationDecision::Accepted { .. }) {
            sanitized_crash_report = None;
        }
        Some(decision)
    } else {
        None
    };
    Ok(ProcessOutcome {
        primary_exit_code,
        user_cancelled,
        process_exit_codes: exits,
        log_tail,
        dropped_log_lines,
        sanitized_crash_report,
        crash_deduplication,
    })
}

fn process_specs(snapshot: &WorkspaceSnapshot) -> Result<Vec<ProcessSpec>, FusionError> {
    let manifest = &snapshot.manifest;
    if let Some(runtime) = &manifest.runtime {
        return Ok(vec![ProcessSpec {
            id: "runtime".into(),
            executable: runtime.executable.clone(),
            args: runtime.args.clone(),
            env: runtime.env.clone(),
            depends_on: Vec::new(),
            role: ProcessRole::Primary,
        }]);
    }
    if let Some(process) = &manifest.launch {
        return Ok(vec![process.clone()]);
    }
    let graph = manifest.orchestrator.as_ref().ok_or_else(|| {
        FusionError::new(ErrorCode::InvalidManifest, "Manifest has no launch graph.")
    })?;
    let mut pending: BTreeMap<_, _> = graph
        .processes
        .iter()
        .map(|spec| (spec.id.clone(), spec.clone()))
        .collect();
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::new();
    while !pending.is_empty() {
        let id = pending
            .iter()
            .find(|(_, spec)| spec.depends_on.iter().all(|id| emitted.contains(id)))
            .map(|(id, _)| id.clone())
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Process graph contains a dependency cycle.",
                )
            })?;
        emitted.insert(id.clone());
        ordered.push(pending.remove(&id).expect("selected pending process"));
    }
    Ok(ordered)
}

fn prepare_command(
    spec: &ProcessSpec,
    workspace: &Path,
    isolated: &BTreeMap<String, String>,
) -> Result<Command, FusionError> {
    let relative = super::fs_safety::validate_relative_path(&spec.executable)?;
    let executable = workspace.join(relative);
    let basename = executable
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if [
        "cmd.exe",
        "cmd",
        "powershell.exe",
        "powershell",
        "pwsh.exe",
        "pwsh",
        "sh",
        "bash",
        "dash",
        "zsh",
        "wscript.exe",
        "cscript.exe",
        "mshta.exe",
    ]
    .contains(&basename.as_str())
        || ["bat", "cmd", "ps1", "sh", "vbs", "js"].contains(
            &executable
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
        )
    {
        return Err(FusionError::new(
            ErrorCode::ProcessLaunchFailed,
            "Shell and script-host execution is prohibited.",
        )
        .with_detail("processId", &spec.id));
    }
    let mut variables = isolated.clone();
    variables.insert(
        "WORKSPACE_DIR".into(),
        workspace
            .to_str()
            .ok_or_else(|| {
                FusionError::new(ErrorCode::InvalidPath, "Workspace path is not Unicode.")
            })?
            .to_owned(),
    );
    let mut command = Command::new(executable);
    command.current_dir(workspace).env_clear();
    // Minimal OS environment; no launcher tokens, proxy credentials, Python
    // injection variables, or untrusted search-path entries are inherited.
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command
            .env("SystemRoot", &system_root)
            .env("WINDIR", &system_root)
            .env(
                "PATH",
                std::path::PathBuf::from(system_root).join("System32"),
            );
    }
    for argument in &spec.args {
        command.arg(expand(argument, &variables)?);
    }
    for (name, value) in &spec.env {
        let upper = name.to_ascii_uppercase();
        let value = expand(value, &variables)?;
        if matches!(upper.as_str(), "APPDATA" | "LOCALAPPDATA" | "TEMP" | "TMP")
            && variables.get(&upper) != Some(&value)
        {
            return Err(FusionError::new(
                ErrorCode::InvalidEnvironment,
                "Manifest cannot override isolated save or temporary paths.",
            )
            .with_detail("variable", name));
        }
        command.env(upper, value);
    }
    for (name, value) in isolated {
        command.env(name, value);
    }
    Ok(command)
}

fn expand(value: &str, variables: &BTreeMap<String, String>) -> Result<String, FusionError> {
    if value.contains('\0') {
        return Err(FusionError::new(
            ErrorCode::InvalidEnvironment,
            "NUL in process parameter.",
        ));
    }
    let mut result = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("{{") {
        if remaining[..start].contains("}}") {
            return Err(FusionError::new(
                ErrorCode::InvalidEnvironment,
                "Unmatched process variable delimiter.",
            ));
        }
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 2..];
        let end = remaining.find("}}").ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidEnvironment, "Unclosed process variable.")
        })?;
        let name = &remaining[..end];
        let value = variables.get(name).ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidEnvironment,
                "Unknown or unavailable process variable.",
            )
            .with_detail("variable", name)
        })?;
        result.push_str(value);
        remaining = &remaining[end + 2..];
        if result.len() > 1024 * 1024 {
            return Err(FusionError::new(
                ErrorCode::InvalidEnvironment,
                "Process parameter exceeds size limit.",
            ));
        }
    }
    if remaining.contains("}}") {
        return Err(FusionError::new(
            ErrorCode::InvalidEnvironment,
            "Unmatched process variable delimiter.",
        ));
    }
    result.push_str(remaining);
    Ok(result)
}

/// Chunk framing bounds partial lines independently of the shared ring. A
/// multi-gigabyte stream without newlines never grows any allocation with it.
struct LineAccumulator {
    pending: Vec<u8>,
    truncated: bool,
    limit: usize,
}
impl LineAccumulator {
    fn consume(
        &mut self,
        bytes: &[u8],
        id: &str,
        stream: LogStream,
        logs: &Mutex<BoundedLogRingBuffer>,
    ) {
        for &byte in bytes {
            if byte == b'\n' {
                self.emit(id, stream, logs);
            } else if self.pending.len() < self.limit {
                self.pending.push(byte);
            } else {
                self.truncated = true;
            }
        }
    }
    fn emit(&mut self, id: &str, stream: LogStream, logs: &Mutex<BoundedLogRingBuffer>) {
        if self.pending.last() == Some(&b'\r') {
            self.pending.pop();
        }
        lock_logs(logs).push(LogLine {
            process_id: id.to_owned(),
            stream,
            text: String::from_utf8_lossy(&self.pending).into_owned(),
            truncated: self.truncated,
        });
        self.pending.clear();
        self.truncated = false;
    }
}
#[cfg(windows)]
fn reader(
    mut file: File,
    id: String,
    stream: LogStream,
    logs: Arc<Mutex<BoundedLogRingBuffer>>,
    limits: LogLimits,
) -> Result<std::thread::JoinHandle<()>, FusionError> {
    std::thread::Builder::new()
        .name("fusion-process-log".into())
        .spawn(move || {
            let mut accumulator = LineAccumulator {
                pending: Vec::with_capacity(limits.max_line_bytes.min(4096)),
                truncated: false,
                limit: limits.max_line_bytes,
            };
            let mut bytes = [0_u8; 4096];
            loop {
                match file.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => accumulator.consume(&bytes[..count], &id, stream, &logs),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            if !accumulator.pending.is_empty() || accumulator.truncated {
                accumulator.emit(&id, stream, &logs);
            }
        })
        .map_err(|error| {
            FusionError::new(
                ErrorCode::ProcessLaunchFailed,
                "Cannot start bounded process log reader.",
            )
            .with_detail("osError", error.to_string())
        })
}

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod tests;
