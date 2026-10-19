//! Disk-streaming archive extraction and explicitly configured, hash-pinned patch tools.
//!
//! This module never discovers/downloads executable tools or executes a shell. Patch
//! inputs are copied into a private directory before a trusted tool sees them; even
//! a tool that modifies its source cannot modify a CAS hardlink or external game.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::cas::{verify_digest, verify_open_digest};
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::schema::{ArchiveFormat, DigestAlgorithm, DigestSpec, PatchFormat, PatchSpec};

/// Limits apply to both declared archive sizes and bytes actually streamed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtractionLimits {
    pub max_entries: usize,
    pub max_uncompressed_bytes: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_entries: 250_000,
            max_uncompressed_bytes: 1 << 40,
        }
    }
}

#[derive(Debug, Default)]
pub struct ArchiveTransformer {
    limits: ExtractionLimits,
}

impl ArchiveTransformer {
    pub fn new(limits: ExtractionLimits) -> Self {
        Self { limits }
    }

    /// Extract an explicit format into empty staging. CAS container names need no
    /// extension. Failure leaves staging inactive; its owner decides when to clean it.
    pub fn extract(
        &self,
        archive_path: &Path,
        format: ArchiveFormat,
        staging_root: &Path,
    ) -> Result<(), FusionError> {
        if format == ArchiveFormat::Raw {
            return Err(FusionError::new(
                ErrorCode::ArchiveInvalid,
                "Raw layers require an explicit target filename via extract_raw.",
            ));
        }
        let _root_guard = fs_safety::ensure_directory(staging_root)?;
        if fs::read_dir(staging_root)
            .map_err(|error| io_error("inspect extraction staging", error))?
            .next()
            .is_some()
        {
            return Err(FusionError::new(
                ErrorCode::ArchiveInvalid,
                "Archive extraction requires an empty private staging directory.",
            ));
        }
        // Retain the exact input descriptor, with deny-write sharing on Windows.
        let _source_parent = fs_safety::guard(archive_path.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Archive input requires a parent directory.",
            )
        })?)?;
        let file = fs_safety::open_read(archive_path)?;
        match format {
            ArchiveFormat::Zip => self.extract_zip(file, staging_root),
            ArchiveFormat::SevenZip => self.extract_seven_zip(file, staging_root),
            ArchiveFormat::Raw => unreachable!(),
        }
    }

    /// A raw object has no archive entry names; the caller supplies a validated,
    /// resolved output path. The output is an exclusive, independent physical copy.
    pub fn extract_raw(&self, source: &Path, target: &Path) -> Result<(), FusionError> {
        let _source_parent = fs_safety::guard(source.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Raw layer requires a parent directory.",
            )
        })?)?;
        let file = fs_safety::open_read(source)?;
        let size = file
            .metadata()
            .map_err(|error| io_error("inspect raw layer", error))?
            .len();
        if self.limits.max_entries == 0 || size > self.limits.max_uncompressed_bytes {
            return Err(limit_error());
        }
        fs_safety::copy_file(source, target)?;
        Ok(())
    }

    fn extract_zip(&self, source: File, root: &Path) -> Result<(), FusionError> {
        let mut archive = zip::ZipArchive::new(source).map_err(archive_error)?;
        let mut inventory = ArchiveInventory::new(self.limits);
        // Preflight every header before creating the first output file.
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(archive_error)?;
            if entry.is_symlink()
                || entry.unix_mode().is_some_and(|mode| {
                    let kind = mode & 0o170000;
                    kind != 0 && kind != 0o100000 && kind != 0o040000
                })
            {
                return Err(archive_error(
                    "Archive links and special files are forbidden.",
                ));
            }
            inventory.add(entry.name(), entry.is_dir(), entry.size())?;
        }
        let mut total = 0;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(archive_error)?;
            let relative = archive_relative(entry.name(), entry.is_dir())?;
            let output = root.join(relative);
            if entry.is_dir() {
                let _directory_guard = fs_safety::ensure_directory(&output)?;
                continue;
            }
            let expected = entry.size();
            let _parent_guard = fs_safety::ensure_directory(
                output
                    .parent()
                    .ok_or_else(|| archive_error("Archive output has no parent directory."))?,
            )?;
            let mut destination = create_output(&output)?;
            copy_bounded(
                &mut entry,
                &mut destination,
                expected,
                &mut total,
                self.limits.max_uncompressed_bytes,
            )?;
            destination
                .sync_all()
                .map_err(|error| io_error("sync extracted file", error))?;
        }
        Ok(())
    }

    fn extract_seven_zip(&self, source: File, root: &Path) -> Result<(), FusionError> {
        let length = source
            .metadata()
            .map_err(|error| io_error("inspect 7z archive", error))?
            .len();
        let mut archive =
            sevenz_rust::SevenZReader::new(source, length, sevenz_rust::Password::empty())
                .map_err(archive_error)?;
        let mut inventory = ArchiveInventory::new(self.limits);
        for entry in &archive.archive().files {
            validate_seven_zip_attributes(entry)?;
            inventory.add(entry.name(), entry.is_directory(), entry.size())?;
        }
        let mut total = 0;
        // Preserve structured filesystem/limit errors through the decoder callback.
        let mut failure = None;
        let result = archive.for_each_entries(|entry, reader| {
            let mut extract = || -> Result<(), FusionError> {
                let relative = archive_relative(entry.name(), entry.is_directory())?;
                let output = root.join(relative);
                if entry.is_directory() {
                    let _directory_guard = fs_safety::ensure_directory(&output)?;
                } else {
                    let _parent_guard =
                        fs_safety::ensure_directory(output.parent().ok_or_else(|| {
                            archive_error("Archive output has no parent directory.")
                        })?)?;
                    let mut destination = create_output(&output)?;
                    copy_bounded(
                        reader,
                        &mut destination,
                        entry.size(),
                        &mut total,
                        self.limits.max_uncompressed_bytes,
                    )?;
                    destination
                        .sync_all()
                        .map_err(|error| io_error("sync extracted file", error))?;
                }
                Ok(())
            };
            match extract() {
                Ok(()) => Ok(true),
                Err(error) => {
                    let text = error.to_string();
                    failure = Some(error);
                    Err(sevenz_rust::Error::other(text))
                }
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        result.map_err(archive_error)
    }
}

/// Reject reparse points, Unix links/special files encoded in the high attribute
/// word, and anti-items (deletion directives). Do not apply archive permissions.
fn validate_seven_zip_attributes(
    entry: &sevenz_rust::SevenZArchiveEntry,
) -> Result<(), FusionError> {
    let kind = (entry.windows_attributes >> 16) & 0o170000;
    if entry.is_anti_item()
        || (entry.has_windows_attributes
            && (entry.windows_attributes & 0x400 != 0
                || (kind != 0 && kind != 0o100000 && kind != 0o040000)))
    {
        return Err(archive_error(
            "7z links, special files and anti-items are forbidden.",
        ));
    }
    Ok(())
}

struct ArchiveInventory {
    paths: BTreeMap<String, bool>,
    limits: ExtractionLimits,
    entries: usize,
    bytes: u64,
}

impl ArchiveInventory {
    fn new(limits: ExtractionLimits) -> Self {
        Self {
            paths: BTreeMap::new(),
            limits,
            entries: 0,
            bytes: 0,
        }
    }

    fn add(&mut self, name: &str, directory: bool, size: u64) -> Result<(), FusionError> {
        let relative = archive_relative(name, directory)?;
        let key = relative.to_string_lossy().replace('\\', "/").to_lowercase();
        if self.paths.contains_key(&key) {
            return Err(archive_error(
                "Archive contains duplicate or case-colliding paths.",
            ));
        }
        // A file cannot be the ancestor of another entry, regardless of header order.
        let mut ancestor = key.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if self.paths.get(parent) == Some(&false) {
                return Err(archive_error("Archive file and directory paths conflict."));
            }
            ancestor = parent;
        }
        let prefix = format!("{key}/");
        if !directory
            && self
                .paths
                .range(prefix.clone()..)
                .next()
                .is_some_and(|(other, _)| other.starts_with(&prefix))
        {
            return Err(archive_error("Archive file and directory paths conflict."));
        }
        self.entries = self.entries.checked_add(1).ok_or_else(limit_error)?;
        self.bytes = self.bytes.checked_add(size).ok_or_else(limit_error)?;
        if self.entries > self.limits.max_entries || self.bytes > self.limits.max_uncompressed_bytes
        {
            return Err(limit_error());
        }
        self.paths.insert(key, directory);
        Ok(())
    }
}

fn archive_relative(name: &str, directory: bool) -> Result<PathBuf, FusionError> {
    let name = if directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    fs_safety::validate_relative_path(name)
}

fn create_output(path: &Path) -> Result<File, FusionError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error("exclusively create extracted file", error))
}

fn copy_bounded(
    reader: &mut (impl Read + ?Sized),
    writer: &mut impl Write,
    expected: u64,
    total: &mut u64,
    limit: u64,
) -> Result<(), FusionError> {
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(archive_error)?;
        if count == 0 {
            if bytes != expected {
                return Err(archive_error(
                    "Archive entry length differs from its header.",
                ));
            }
            return Ok(());
        }
        bytes = bytes.checked_add(count as u64).ok_or_else(limit_error)?;
        *total = total.checked_add(count as u64).ok_or_else(limit_error)?;
        if bytes > expected || *total > limit {
            return Err(limit_error());
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| io_error("write extracted file", error))?;
    }
}

#[derive(Clone, Debug)]
pub struct PinnedPatchTool {
    pub executable: PathBuf,
    pub sha256: String,
    pub version: String,
    pub format: PatchFormat,
}

/// Checked bytes and every parent path stay protected for the registry lifetime.
pub(crate) struct VerifiedPatchRegistration {
    file: Mutex<File>,
    _parent: fs_safety::DirectoryGuard,
    blake3: String,
    sha256: String,
    executable: PathBuf,
    format: PatchFormat,
    version: String,
}

impl std::fmt::Debug for VerifiedPatchRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedPatchRegistration")
            .field("executable", &self.executable)
            .field("sha256", &self.sha256)
            .finish_non_exhaustive()
    }
}

impl VerifiedPatchRegistration {
    /// Compute both digests from one no-follow descriptor without executing it.
    pub(crate) fn inspect(tool: &PinnedPatchTool) -> Result<Self, FusionError> {
        use sha2::{Digest, Sha256};
        let parent = fs_safety::guard(tool.executable.parent().ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidPath, "Patch tool has no parent.")
        })?)?;
        let mut file = fs_safety::open_read(&tool.executable)?;
        let mut sha256 = Sha256::new();
        let mut blake3 = blake3::Hasher::new();
        let mut bytes = [0; 64 * 1024];
        loop {
            let count = file
                .read(&mut bytes)
                .map_err(|error| io_error("Hash patch tool", error))?;
            if count == 0 {
                break;
            }
            sha256.update(&bytes[..count]);
            blake3.update(&bytes[..count]);
        }
        let sha256 = hex::encode(sha256.finalize());
        if !sha256.eq_ignore_ascii_case(&tool.sha256) {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Patch-tool SHA-256 differs from its approved build.",
            ));
        }
        Ok(Self {
            file: Mutex::new(file),
            _parent: parent,
            blake3: blake3.finalize().to_hex().to_string(),
            sha256,
            executable: tool.executable.clone(),
            format: tool.format,
            version: tool.version.clone(),
        })
    }

    pub(crate) fn blake3(&self) -> &str {
        &self.blake3
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchReceipt {
    pub tool_version: String,
    pub tool_sha256: String,
    pub format: PatchFormat,
    pub output_digest: DigestSpec,
}

/// Registration supplies trusted application configuration, never a source URL.
#[derive(Debug, Clone, Default)]
pub struct PatchToolRegistry {
    tools: Vec<PinnedPatchTool>,
    verified: Vec<Arc<VerifiedPatchRegistration>>,
    #[cfg(test)]
    test_prelude: Vec<String>,
}

impl PatchToolRegistry {
    pub fn configured_formats(&self) -> Vec<PatchFormat> {
        self.verified_formats()
    }

    pub fn verified_formats(&self) -> Vec<PatchFormat> {
        self.tools
            .iter()
            .filter(|tool| self.registration(tool).is_some())
            .map(|tool| tool.format)
            .collect()
    }

    fn registration(&self, tool: &PinnedPatchTool) -> Option<&Arc<VerifiedPatchRegistration>> {
        self.verified.iter().find(|entry| {
            entry.executable == tool.executable
                && entry.sha256.eq_ignore_ascii_case(&tool.sha256)
                && entry.format == tool.format
                && entry.version == tool.version
        })
    }

    pub(crate) fn register_verified(
        &mut self,
        tool: PinnedPatchTool,
        registration: VerifiedPatchRegistration,
    ) -> Result<(), FusionError> {
        // Apply all structural and ambiguity validation to host registrations too.
        let mut tools = self.tools.clone();
        tools.push(tool.clone());
        Self::new(tools)?;
        if registration.executable != tool.executable
            || !registration.sha256.eq_ignore_ascii_case(&tool.sha256)
            || registration.format != tool.format
            || registration.version != tool.version
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Patch registration differs from inspected tool.",
            ));
        }
        self.tools.push(tool);
        self.verified.push(Arc::new(registration));
        Ok(())
    }

    /// Compatibility for trusted host callers. A true flag still verifies and
    /// retains each exact configured file; source manifests cannot call this API.
    pub fn verify_explicit_trust(mut self, trusted: bool) -> Result<Self, FusionError> {
        self.verified.clear();
        if trusted {
            for tool in &self.tools {
                self.verified
                    .push(Arc::new(VerifiedPatchRegistration::inspect(tool)?));
            }
        }
        Ok(self)
    }
    pub fn new(tools: Vec<PinnedPatchTool>) -> Result<Self, FusionError> {
        for (index, tool) in tools.iter().enumerate() {
            if !tool.executable.is_absolute()
                || tool.version.trim().is_empty()
                || tool.version.chars().any(char::is_control)
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Patch tools require an absolute executable path and pinned version.",
                ));
            }
            validate_digest(&DigestSpec {
                algorithm: DigestAlgorithm::Sha256,
                value: tool.sha256.clone(),
            })?;
            if tools[..index]
                .iter()
                .any(|other| other.format == tool.format)
            {
                return Err(FusionError::new(
                    ErrorCode::DuplicateId,
                    "Only one pinned patch tool may be registered per format.",
                ));
            }
        }
        Ok(Self {
            tools,
            verified: Vec::new(),
            #[cfg(test)]
            test_prelude: Vec::new(),
        })
    }

    /// The caller resolves a workspace-owned destination and supplies exact-hash
    /// user trust. An existing destination must first be detached by its owner.
    /// This adapter never replaces a preexisting file, including a CAS hardlink.
    pub fn apply(
        &self,
        spec: &PatchSpec,
        input: &Path,
        patch: &Path,
        output: &Path,
        trusted: bool,
        timeout: Duration,
    ) -> Result<PatchReceipt, FusionError> {
        if !trusted {
            return Err(FusionError::new(
                ErrorCode::UntrustedExecutable,
                "Explicit trust is required for the pinned patch-tool hash.",
            ));
        }
        // Retained as a compatibility wrapper; production assembly uses the
        // per-format registration API and never accepts a blanket trust flag.
        self.clone()
            .verify_explicit_trust(true)?
            .apply_verified(spec, input, patch, output, timeout)
    }

    pub fn apply_verified(
        &self,
        spec: &PatchSpec,
        input: &Path,
        patch: &Path,
        output: &Path,
        timeout: Duration,
    ) -> Result<PatchReceipt, FusionError> {
        fs_safety::validate_relative_path(&spec.input_path)?;
        fs_safety::validate_relative_path(&spec.output_path)?;
        validate_digest(&spec.input_digest)?;
        validate_digest(&spec.output_digest)?;
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.format == spec.format)
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::MissingPatchTool,
                    "No verified patch tool is configured for this format.",
                )
                .with_detail("format", spec.format)
            })?;
        let registration = self.registration(tool).ok_or_else(|| {
            FusionError::new(
                ErrorCode::UntrustedExecutable,
                "This patch format has no verified host-tool registration.",
            )
        })?;
        let mut retained = registration.file.lock().map_err(|_| {
            FusionError::new(
                ErrorCode::IntegrityViolation,
                "Patch-tool descriptor lock failed.",
            )
        })?;
        verify_open_digest(
            &mut retained,
            &DigestSpec {
                algorithm: DigestAlgorithm::Sha256,
                value: registration.sha256.clone(),
            },
        )?;
        verify_open_digest(
            &mut retained,
            &DigestSpec {
                algorithm: DigestAlgorithm::Blake3,
                value: registration.blake3.clone(),
            },
        )?;
        // Holding this no-follow descriptor prevents mutation or replacement of
        // the hash-verified tool until the child has exited on Windows.
        let _tool_parent = fs_safety::guard(tool.executable.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Patch tool requires a parent directory.",
            )
        })?)?;
        let mut _tool_file = fs_safety::open_read(&tool.executable)?;
        verify_open_digest(
            &mut _tool_file,
            &DigestSpec {
                algorithm: DigestAlgorithm::Sha256,
                value: tool.sha256.to_ascii_lowercase(),
            },
        )?;
        let parent = output.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Patch output requires a parent directory.",
            )
        })?;
        let _parent_guard = fs_safety::ensure_directory(parent)?;
        if fs::symlink_metadata(output).is_ok() {
            return Err(FusionError::new(
                ErrorCode::ProjectionUnavailable,
                "Patch output must be a new file, detached from any existing projection.",
            ));
        }
        let private = PrivatePatchDirectory::allocate(parent)?;
        let _private_guard = fs_safety::guard(&private.path)?;
        let private_input = private.path.join("input.bin");
        let private_patch = private.path.join("patch.bin");
        let private_output = private.path.join("output.bin");
        fs_safety::copy_file(input, &private_input)?;
        fs_safety::copy_file(patch, &private_patch)?;
        verify_digest(&private_input, &spec.input_digest)?;
        // Both paths are independent physical copies, not links to caller data.
        let mut command = Command::new(&tool.executable);
        #[cfg(test)]
        command.args(&self.test_prelude);
        command.current_dir(&private.path).env_clear();
        // xdelta3 accepts options from XDELTA in the ambient environment. Do
        // not inherit any tool options, PATH entries or preload variables.
        #[cfg(windows)]
        for name in ["SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("TEMP", &private.path).env("TMP", &private.path);
        match tool.format {
            PatchFormat::Xdelta3 => {
                command
                    .args(["-d", "-s"])
                    .arg(&private_input)
                    .arg(&private_patch)
                    .arg(&private_output);
            }
            PatchFormat::Bsdiff => {
                command
                    .arg(&private_input)
                    .arg(&private_output)
                    .arg(&private_patch);
            }
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        run_patch_tool(&mut command, timeout)?;
        let _output_file = fs_safety::open_read(&private_output)?;
        let output_identity = fs_safety::file_identity(&private_output)?;
        if [input, patch, &private_input, &private_patch]
            .into_iter()
            .map(fs_safety::file_identity)
            .collect::<Result<Vec<_>, _>>()?
            .contains(&output_identity)
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Patch tool output must not alias caller input files.",
            ));
        }
        verify_digest(&private_output, &spec.output_digest)?;
        drop(_output_file);
        publish_output(&private_output, output, &spec.output_digest)?;
        Ok(PatchReceipt {
            tool_version: tool.version.clone(),
            tool_sha256: tool.sha256.to_ascii_lowercase(),
            format: tool.format,
            output_digest: spec.output_digest.clone(),
        })
    }
}

fn run_patch_tool(command: &mut Command, timeout: Duration) -> Result<(), FusionError> {
    let mut child = command
        .spawn()
        .map_err(|error| io_error("start pinned patch tool", error))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(FusionError::new(
                    ErrorCode::PatchToolFailed,
                    "Pinned patch tool exited unsuccessfully.",
                )
                .with_detail("exitCode", status.code()));
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io_error("wait for patch tool", error));
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            child
                .wait()
                .map_err(|error| io_error("reap timed-out patch tool", error))?;
            return Err(FusionError::new(
                ErrorCode::PatchToolTimeout,
                "Pinned patch tool exceeded its execution timeout.",
            ));
        }
        std::thread::sleep(Duration::from_millis(10).min(timeout - started.elapsed().min(timeout)));
    }
}

fn publish_output(
    source: &Path,
    destination: &Path,
    digest: &DigestSpec,
) -> Result<(), FusionError> {
    #[cfg(windows)]
    {
        let mut file = crate::install_ownership::open_publication_file(source)
            .map_err(|error| io_error("open patch output for publication", error))?;
        // The retained publication descriptor denies writes/replacement while
        // this second verification and handle-bound, no-replace rename run.
        verify_open_digest(&mut file, digest)?;
        crate::install_ownership::publish_rename_no_replace(&file, destination)
            .map_err(|error| io_error("publish verified patch output", error))?;
    }
    #[cfg(not(windows))]
    {
        verify_digest(source, digest)?;
        // link(2) is atomic and refuses an existing destination; the temporary
        // inode is already physically separate from every caller-owned input.
        fs::hard_link(source, destination)
            .map_err(|error| io_error("publish verified patch output", error))?;
        fs::remove_file(source).map_err(|error| io_error("remove patch staging file", error))?;
    }
    Ok(())
}

struct PrivatePatchDirectory {
    path: PathBuf,
}

impl PrivatePatchDirectory {
    fn allocate(parent: &Path) -> Result<Self, FusionError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| io_error("allocate patch staging", error))?
            .as_nanos();
        loop {
            let counter = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".fusion-patch-{stamp:x}-{counter:x}"));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("allocate patch staging", error)),
            }
        }
    }
}

impl Drop for PrivatePatchDirectory {
    fn drop(&mut self) {
        // Cleanup is deliberately limited to the exclusively allocated private
        // directory and delegates link/reparse checks to the shared safe helper.
        let _ = fs_safety::safe_remove_tree(&self.path);
    }
}

fn validate_digest(digest: &DigestSpec) -> Result<(), FusionError> {
    if digest.value.len() != 64 || !digest.value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FusionError::new(
            ErrorCode::InvalidDigest,
            "Expected a 64-digit BLAKE3 or SHA-256 digest.",
        ));
    }
    Ok(())
}

fn archive_error(error: impl std::fmt::Display) -> FusionError {
    FusionError::new(ErrorCode::ArchiveInvalid, error.to_string())
}

fn limit_error() -> FusionError {
    FusionError::new(
        ErrorCode::ExtractionLimitExceeded,
        "Archive exceeds the configured file-count or uncompressed-byte limit.",
    )
}

fn io_error(operation: &str, error: impl std::fmt::Display) -> FusionError {
    FusionError::new(ErrorCode::Io, format!("Cannot {operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::super::cas::{hash_file, CasStore};
    use super::*;
    use zip::write::SimpleFileOptions;

    fn zip_fixture(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn zip_extracts_nested_files_to_independent_writable_outputs() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("layer-with-no-extension");
        let root = directory.path().join("workspace");
        zip_fixture(
            &archive,
            &[("bin/runtime.exe", b"runtime"), ("readme.txt", b"notes")],
        );
        let before = hash_file(&archive, DigestAlgorithm::Blake3).unwrap();
        ArchiveTransformer::default()
            .extract(&archive, ArchiveFormat::Zip, &root)
            .unwrap();
        assert_eq!(fs::read(root.join("bin/runtime.exe")).unwrap(), b"runtime");
        assert_eq!(fs::read(root.join("readme.txt")).unwrap(), b"notes");
        assert!(!root
            .join("bin/runtime.exe")
            .metadata()
            .unwrap()
            .permissions()
            .readonly());
        assert_eq!(
            hash_file(&archive, DigestAlgorithm::Blake3).unwrap(),
            before
        );
    }

    #[test]
    fn zip_preflight_rejects_traversal_before_writing_any_entry() {
        for invalid in [
            "../sentinel",
            "nested/../../sentinel",
            "C:/sentinel",
            "nested\\sentinel",
            "NUL.txt",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let archive = directory.path().join("layer.zip");
            let root = directory.path().join("workspace");
            fs::write(directory.path().join("sentinel"), b"external user file").unwrap();
            zip_fixture(
                &archive,
                &[("valid-first", b"must not appear"), (invalid, b"invalid")],
            );
            let error = ArchiveTransformer::default()
                .extract(&archive, ArchiveFormat::Zip, &root)
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidPath);
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
            assert_eq!(
                fs::read(directory.path().join("sentinel")).unwrap(),
                b"external user file"
            );
        }
    }

    #[test]
    fn zip_rejects_symlink_entries_without_following_target() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("links.zip");
        let root = directory.path().join("workspace");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        writer
            .add_symlink("game.exe", "../sentinel", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        fs::write(directory.path().join("sentinel"), b"outside").unwrap();
        let error = ArchiveTransformer::default()
            .extract(&archive, ArchiveFormat::Zip, &root)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ArchiveInvalid);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        assert_eq!(
            fs::read(directory.path().join("sentinel")).unwrap(),
            b"outside"
        );
    }

    #[test]
    fn zip_rejects_case_collisions_and_file_directory_conflicts() {
        for names in [
            ["bin/Game.exe", "BIN/game.EXE"],
            ["bin", "bin/game.exe"],
            ["bin/game.exe", "bin"],
        ] {
            let directory = tempfile::tempdir().unwrap();
            let archive = directory.path().join("collision.zip");
            let root = directory.path().join("workspace");
            zip_fixture(&archive, &[(names[0], b"first"), (names[1], b"second")]);
            let error = ArchiveTransformer::default()
                .extract(&archive, ArchiveFormat::Zip, &root)
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::ArchiveInvalid);
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        }
    }

    #[test]
    fn archive_limits_reject_before_materializing_and_stream_limits_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("limited.zip");
        zip_fixture(&archive, &[("first", b"123"), ("second", b"456")]);
        for (index, limits) in [
            ExtractionLimits {
                max_entries: 1,
                max_uncompressed_bytes: 99,
            },
            ExtractionLimits {
                max_entries: 9,
                max_uncompressed_bytes: 5,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let root = directory.path().join(format!("output-{index}"));
            assert_eq!(
                ArchiveTransformer::new(limits)
                    .extract(&archive, ArchiveFormat::Zip, &root)
                    .unwrap_err()
                    .code,
                ErrorCode::ExtractionLimitExceeded
            );
            assert_eq!(fs::read_dir(root).unwrap().count(), 0);
        }
        let mut total = 0;
        let error = copy_bounded(
            &mut &b"exceeds declared length"[..],
            &mut Vec::new(),
            3,
            &mut total,
            99,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::ExtractionLimitExceeded);
        let mut total = 0;
        assert_eq!(
            copy_bounded(&mut &b"short"[..], &mut Vec::new(), 8, &mut total, 99)
                .unwrap_err()
                .code,
            ErrorCode::ArchiveInvalid
        );
    }

    #[test]
    fn extraction_preserves_existing_staging_and_raw_creates_physical_copy() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("layer.zip");
        let root = directory.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("sentinel"), b"user file").unwrap();
        zip_fixture(&archive, &[("sentinel", b"replacement")]);
        assert_eq!(
            ArchiveTransformer::default()
                .extract(&archive, ArchiveFormat::Zip, &root)
                .unwrap_err()
                .code,
            ErrorCode::ArchiveInvalid
        );
        assert_eq!(fs::read(root.join("sentinel")).unwrap(), b"user file");
        assert_eq!(
            ArchiveTransformer::default()
                .extract(
                    &archive,
                    ArchiveFormat::Raw,
                    &directory.path().join("raw-output")
                )
                .unwrap_err()
                .code,
            ErrorCode::ArchiveInvalid
        );
        let source = directory.path().join("raw-game");
        fs::write(&source, b"raw data").unwrap();
        let target = root.join("raw-copy");
        ArchiveTransformer::default()
            .extract_raw(&source, &target)
            .unwrap();
        assert_ne!(
            fs_safety::file_identity(&source).unwrap(),
            fs_safety::file_identity(&target).unwrap()
        );
        fs::write(&target, b"mutable copy").unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"raw data");
        assert!(ArchiveTransformer::default()
            .extract_raw(&source, &target)
            .is_err());
        let limits = ExtractionLimits {
            max_entries: 1,
            max_uncompressed_bytes: 1,
        };
        assert_eq!(
            ArchiveTransformer::new(limits)
                .extract_raw(&source, &root.join("too-large"))
                .unwrap_err()
                .code,
            ErrorCode::ExtractionLimitExceeded
        );
        assert!(!root.join("too-large").exists());
    }

    #[test]
    fn seven_zip_attribute_and_inventory_checks_reject_links_anti_items_and_overflow() {
        let mut entry = sevenz_rust::SevenZArchiveEntry::new();
        entry.has_windows_attributes = true;
        entry.windows_attributes = 0x400;
        assert!(validate_seven_zip_attributes(&entry).is_err());
        entry.windows_attributes = 0o120000 << 16;
        assert!(validate_seven_zip_attributes(&entry).is_err());
        entry.windows_attributes = 0;
        entry.is_anti_item = true;
        assert!(validate_seven_zip_attributes(&entry).is_err());
        let mut inventory = ArchiveInventory::new(ExtractionLimits {
            max_entries: usize::MAX,
            max_uncompressed_bytes: u64::MAX,
        });
        inventory.add("first", false, u64::MAX).unwrap();
        assert_eq!(
            inventory.add("second", false, 1).unwrap_err().code,
            ErrorCode::ExtractionLimitExceeded
        );
    }

    // This fixture tests process arguments, pinning, physical isolation, timeout
    // and atomic publication. Its literal output is NOT an xdelta/bsdiff codec.
    const PATCH_HELPER: &str = r#"import pathlib, sys, time
mode = sys.argv[1]
args = sys.argv[2:]
if args[:2] == ['-d', '-s']:
    assert len(args) == 5
    source, patch, output = map(pathlib.Path, args[2:])
else:
    assert len(args) == 3
    source, output, patch = map(pathlib.Path, args)
assert source.parent == patch.parent == output.parent == pathlib.Path.cwd()
if mode == 'sleep':
    time.sleep(60)
if mode == 'fail':
    sys.exit(23)
payload = patch.read_bytes()
if mode == 'mutate':
    source.write_bytes(b'tool mutated its private source')
    patch.write_bytes(b'tool mutated its private patch')
if mode == 'alias':
    import os
    os.link(source, output)
else:
    output.write_bytes(payload)
"#;

    fn python_executable() -> PathBuf {
        static PYTHON: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        PYTHON.get_or_init(|| {
            let mut candidates = Vec::new();
            if let Some(value) = std::env::var_os("FUSION_TEST_PYTHON") { candidates.push(PathBuf::from(value)); }
            candidates.extend([PathBuf::from("python3"), PathBuf::from("python")]);
            if let Some(profile) = std::env::var_os("USERPROFILE") {
                candidates.push(PathBuf::from(profile).join(".cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe"));
            }
            for executable in candidates {
                let Ok(output) = Command::new(executable).args(["-I", "-c", "import sys; print(sys.executable)"]).output() else { continue };
                if output.status.success() {
                    let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
                    if path.is_absolute() && path.is_file() { return path; }
                }
            }
            panic!("Patch adapter subprocess tests require Python 3; set FUSION_TEST_PYTHON to an absolute interpreter path.");
        }).clone()
    }

    struct PatchFixture {
        _directory: tempfile::TempDir,
        registry: PatchToolRegistry,
        spec: PatchSpec,
        input: PathBuf,
        patch: PathBuf,
        output: PathBuf,
        library: PathBuf,
    }

    impl PatchFixture {
        fn new(format: PatchFormat, mode: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let original = root.join("original-game.bin");
            fs::write(&original, b"original game bytes").unwrap();
            let library = root.join("library");
            let object = CasStore::open(&library)
                .unwrap()
                .import_file(&original, "game.bin")
                .unwrap();
            let input = object.root.join("game.bin");
            let patch = root.join("patch.bin");
            fs::write(&patch, b"patched game bytes").unwrap();
            let helper = root.join("patch-helper.py");
            fs::write(&helper, PATCH_HELPER).unwrap();
            let executable = python_executable();
            let mut registry = PatchToolRegistry::new(vec![PinnedPatchTool {
                sha256: hash_file(&executable, DigestAlgorithm::Sha256).unwrap(),
                executable,
                version: "literal-protocol-fixture-v1".into(),
                format,
            }])
            .unwrap();
            registry.test_prelude = vec!["-I".into(), helper.to_str().unwrap().into(), mode.into()];
            let spec = PatchSpec {
                format,
                base_layer: "base".into(),
                input_path: "game.bin".into(),
                output_path: "game.bin".into(),
                input_digest: DigestSpec {
                    algorithm: DigestAlgorithm::Blake3,
                    value: hash_file(&input, DigestAlgorithm::Blake3).unwrap(),
                },
                output_digest: DigestSpec {
                    algorithm: DigestAlgorithm::Blake3,
                    value: hash_file(&patch, DigestAlgorithm::Blake3).unwrap(),
                },
            };
            let output = root.join("workspace/patched-game.bin");
            Self {
                _directory: directory,
                registry,
                spec,
                input,
                patch,
                output,
                library,
            }
        }

        fn apply(&self, timeout: Duration) -> Result<PatchReceipt, FusionError> {
            self.registry.apply(
                &self.spec,
                &self.input,
                &self.patch,
                &self.output,
                true,
                timeout,
            )
        }

        fn assert_originals_and_cleanup(&self) {
            assert_eq!(fs::read(&self.input).unwrap(), b"original game bytes");
            assert!(self.input.metadata().unwrap().permissions().readonly());
            assert_eq!(fs::read(&self.patch).unwrap(), b"patched game bytes");
            if let Some(parent) = self.output.parent().filter(|path| path.exists()) {
                assert!(!fs::read_dir(parent).unwrap().any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".fusion-patch-")));
            }
        }
    }

    impl Drop for PatchFixture {
        fn drop(&mut self) {
            let _ = fs_safety::safe_remove_tree(&self.library);
        }
    }

    #[test]
    fn pinned_patch_adapters_use_real_subprocess_protocol_and_isolate_mutating_tool() {
        for format in [PatchFormat::Xdelta3, PatchFormat::Bsdiff] {
            let fixture = PatchFixture::new(format, "mutate");
            let receipt = fixture.apply(Duration::from_secs(15)).unwrap();
            assert_eq!(fs::read(&fixture.output).unwrap(), b"patched game bytes");
            assert_eq!(receipt.format, format);
            assert_eq!(receipt.tool_version, "literal-protocol-fixture-v1");
            assert_eq!(receipt.tool_sha256, fixture.registry.tools[0].sha256);
            assert_ne!(
                fs_safety::file_identity(&fixture.input).unwrap(),
                fs_safety::file_identity(&fixture.output).unwrap()
            );
            fixture.assert_originals_and_cleanup();
        }
    }

    #[test]
    fn patch_adapter_rejects_missing_trust_tool_pin_and_input_or_output_mismatch() {
        let mut fixture = PatchFixture::new(PatchFormat::Xdelta3, "success");
        let error = fixture
            .registry
            .apply(
                &fixture.spec,
                &fixture.input,
                &fixture.patch,
                &fixture.output,
                false,
                Duration::from_secs(15),
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UntrustedExecutable);
        let tool_hash = fixture.registry.tools[0].sha256.clone();
        fixture.registry.tools[0].sha256 = "0".repeat(64);
        assert_eq!(
            fixture.apply(Duration::from_secs(15)).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        fixture.registry.tools[0].sha256 = tool_hash;
        let original_hash = fixture.spec.input_digest.value.clone();
        fixture.spec.input_digest.value = "0".repeat(64);
        assert_eq!(
            fixture.apply(Duration::from_secs(15)).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        fixture.spec.input_digest.value = original_hash;
        fixture.spec.output_digest.value = "0".repeat(64);
        assert_eq!(
            fixture.apply(Duration::from_secs(15)).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        assert!(!fixture.output.exists());
        fixture.assert_originals_and_cleanup();
    }

    #[test]
    fn patch_adapter_reaps_timeout_and_failed_tool_and_cleans_private_files() {
        let fixture = PatchFixture::new(PatchFormat::Xdelta3, "sleep");
        assert_eq!(
            fixture.apply(Duration::from_millis(150)).unwrap_err().code,
            ErrorCode::PatchToolTimeout
        );
        assert!(!fixture.output.exists());
        fixture.assert_originals_and_cleanup();
        let fixture = PatchFixture::new(PatchFormat::Bsdiff, "fail");
        let error = fixture.apply(Duration::from_secs(15)).unwrap_err();
        assert_eq!(error.code, ErrorCode::PatchToolFailed);
        assert_eq!(
            error.details.get("exitCode").unwrap(),
            &serde_json::json!(23)
        );
        fixture.assert_originals_and_cleanup();
    }

    #[test]
    fn patch_adapter_preserves_existing_cas_link_and_rejects_aliased_tool_output() {
        let fixture = PatchFixture::new(PatchFormat::Xdelta3, "success");
        fs::create_dir_all(fixture.output.parent().unwrap()).unwrap();
        fs::hard_link(&fixture.input, &fixture.output).unwrap();
        assert_eq!(
            fixture.apply(Duration::from_secs(15)).unwrap_err().code,
            ErrorCode::ProjectionUnavailable
        );
        fixture.assert_originals_and_cleanup();
        fs_safety::unlink_file(&fixture.output).unwrap();
        let fixture = PatchFixture::new(PatchFormat::Bsdiff, "alias");
        assert_eq!(
            fixture.apply(Duration::from_secs(15)).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        assert!(!fixture.output.exists());
        fixture.assert_originals_and_cleanup();
    }

    #[test]
    fn registry_rejects_ambiguous_or_unpinned_tools() {
        let tool = PinnedPatchTool {
            executable: PathBuf::from("relative-tool.exe"),
            sha256: "0".repeat(64),
            version: "v1".into(),
            format: PatchFormat::Xdelta3,
        };
        assert_eq!(
            PatchToolRegistry::new(vec![tool.clone()]).unwrap_err().code,
            ErrorCode::InvalidManifest
        );
        let mut tool = tool;
        tool.executable = std::env::current_exe().unwrap();
        assert_eq!(
            PatchToolRegistry::new(vec![tool.clone(), tool.clone()])
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        tool.sha256 = "not-a-hash".into();
        assert_eq!(
            PatchToolRegistry::new(vec![tool]).unwrap_err().code,
            ErrorCode::InvalidDigest
        );
    }

    #[test]
    fn registry_advertises_only_retained_per_format_approvals() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fixture.exe");
        fs::write(&executable, b"fixture executable bytes").unwrap();
        let tool = PinnedPatchTool {
            executable,
            sha256: "0".repeat(64),
            version: "v1".into(),
            format: PatchFormat::Xdelta3,
        };
        let registry = PatchToolRegistry::new(vec![tool.clone()]).unwrap();
        assert!(registry.configured_formats().is_empty());
        assert!(registry
            .clone()
            .verify_explicit_trust(false)
            .unwrap()
            .configured_formats()
            .is_empty());
        assert_eq!(
            registry.verify_explicit_trust(true).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        let mut tool = tool;
        tool.sha256 = hash_file(&tool.executable, DigestAlgorithm::Sha256).unwrap();
        let registry = PatchToolRegistry::new(vec![tool])
            .unwrap()
            .verify_explicit_trust(true)
            .unwrap();
        assert_eq!(registry.verified_formats(), vec![PatchFormat::Xdelta3]);
    }
}
