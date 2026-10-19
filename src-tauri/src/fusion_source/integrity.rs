//! Launch-time checks retain the verified descriptors until the process group exits.
//! A digest attests bytes; user trust is an independent decision for that exact digest.

use std::collections::BTreeMap;
use std::fs::File;

use async_trait::async_trait;

use super::canonical::QualifiedGameId;
use super::cas::verify_open_digest;
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::layers::LayerInput;
use super::schema::{CriticalFileRole, CriticalFileSpec, DigestAlgorithm, DigestSpec, LayerSource};
use super::workspace::WorkspaceSnapshot;

/// The adapter must namespace records by Fusion source/game and exact BLAKE3.
/// A matching source, URL, filename, previous version or provider is never trust.
pub trait ExecutableTrust: Send + Sync {
    fn is_trusted(
        &self,
        game: &QualifiedGameId,
        path: &str,
        blake3: &str,
    ) -> Result<bool, FusionError>;
}

/// Must obtain a fresh source index for this check. A cached index, offline
/// status, or a boolean supplied by the caller does not prove strict freshness.
#[async_trait]
pub trait ManifestSyncVerifier: Send + Sync {
    async fn verify_current(
        &self,
        game: &QualifiedGameId,
        manifest_blake3: &str,
    ) -> Result<(), FusionError>;
}

pub struct PreLaunchIntegrityGuard;

/// Retained file bytes and the directory paths through which CreateProcessW
/// resolves them. Keep this owner until every process in the group has exited.
pub struct VerifiedCriticalFiles {
    _files: Vec<File>,
    _directories: Vec<fs_safety::DirectoryGuard>,
}

impl std::fmt::Debug for VerifiedCriticalFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedCriticalFiles")
            .finish_non_exhaustive()
    }
}

impl PreLaunchIntegrityGuard {
    pub fn verify(
        snapshot: &WorkspaceSnapshot,
        trust: &dyn ExecutableTrust,
    ) -> Result<(), FusionError> {
        Self::verify_and_lock(snapshot, trust).map(drop)
    }

    /// Keep these handles alive along with the workspace lease. On Windows,
    /// their share mode excludes writers, renames and deletion of checked bytes.
    pub fn verify_and_lock(
        snapshot: &WorkspaceSnapshot,
        trust: &dyn ExecutableTrust,
    ) -> Result<VerifiedCriticalFiles, FusionError> {
        let (handles, files) = check_integrity(snapshot)?;
        for (path, file) in &files {
            if !trust.is_trusted(&snapshot.game, path, &file.blake3)? {
                return Err(FusionError::new(
                    ErrorCode::UntrustedExecutable,
                    "Explicit trust is required for this exact executable/DLL digest.",
                )
                .with_detail("path", path)
                .with_detail("blake3", &file.blake3));
            }
        }
        Ok(handles)
    }

    /// Assembly/reopening validates bytes without silently granting user trust.
    pub fn verify_integrity(snapshot: &WorkspaceSnapshot) -> Result<(), FusionError> {
        check_integrity(snapshot).map(drop)
    }
}

type CriticalInventory = BTreeMap<String, (String, CriticalFileSpec)>;

fn check_integrity(
    snapshot: &WorkspaceSnapshot,
) -> Result<(VerifiedCriticalFiles, Vec<(String, CriticalFileSpec)>), FusionError> {
    super::evolution::SchemaUpcaster::validate_canonical(&snapshot.manifest)?;
    let mut handles = Vec::new();
    let mut directories = vec![fs_safety::guard(&snapshot.workspace_root)?];
    snapshot.verify_directory_identity()?;
    let mut expected = CriticalInventory::new();
    for id in &snapshot.plan.selected_layers {
        let layer = snapshot.manifest.layers.get(id).ok_or_else(|| {
            FusionError::new(
                ErrorCode::MissingLayer,
                "Critical inventory references an unknown layer.",
            )
        })?;
        let input = snapshot.inputs.get(id).ok_or_else(|| {
            FusionError::new(
                ErrorCode::MissingLayer,
                "Critical inventory has no acquired layer.",
            )
        })?;
        let locator_files = match &layer.source {
            LayerSource::LocalLocator { locator } => {
                let LayerInput::LocalDirectory { root } = input else {
                    return Err(FusionError::new(
                        ErrorCode::LocalBaseModified,
                        "The local base binding is unavailable.",
                    ));
                };
                // Check originals even when a mod replaces the projected path.
                for file in &locator.key_files {
                    let relative = fs_safety::validate_relative_path(&file.path)?;
                    let path = root.join(relative);
                    directories.push(
                        fs_safety::guard(path.parent().expect("key-file parent"))
                            .map_err(|_| local_error(id, &file.path))?,
                    );
                    let mut handle =
                        fs_safety::open_read(&path).map_err(|_| local_error(id, &file.path))?;
                    verify_open_digest(
                        &mut handle,
                        &DigestSpec {
                            algorithm: DigestAlgorithm::Blake3,
                            value: file.blake3.clone(),
                        },
                    )
                    .map_err(|_| local_error(id, &file.path))?;
                    handles.push(handle);
                }
                locator.key_files.as_slice()
            }
            _ => &[],
        };
        let declared = match input {
            LayerInput::Cas {
                runtime_lock: Some(lock),
                ..
            } => &lock.critical_files,
            _ => &layer.critical_files,
        };
        for file in locator_files.iter().chain(declared) {
            let path = super::evolution::mounted_path(&layer.mount_path, &file.path);
            expected.insert(path.to_lowercase(), (path, file.clone()));
        }
    }
    for (relative, file) in expected.values() {
        let projected = snapshot
            .workspace_root
            .join(fs_safety::validate_relative_path(relative)?);
        let missing = matches!(std::fs::symlink_metadata(&projected),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound);
        let code = if missing && file.role == CriticalFileRole::InjectorDll {
            ErrorCode::MissingInjectorDll
        } else {
            ErrorCode::CorruptedCriticalBinary
        };
        let failure = || {
            FusionError::new(
                code,
                if code == ErrorCode::MissingInjectorDll {
                    "A previously assembled injector DLL is missing."
                } else {
                    "A critical executable or library is missing or corrupted."
                },
            )
            .with_detail("path", relative)
            .with_detail("role", file.role)
        };
        directories.push(
            fs_safety::guard(projected.parent().expect("projected parent"))
                .map_err(|_| failure())?,
        );
        let (source, projection_lock) = snapshot
            .lock_critical_projection(relative)
            .map_err(|_| failure())?;
        if let Some(projection_lock) = projection_lock {
            handles.push(projection_lock);
        }
        directories.push(
            fs_safety::guard(source.parent().expect("critical source parent"))
                .map_err(|_| failure())?,
        );
        let mut handle = fs_safety::open_read(&source).map_err(|_| failure())?;
        verify_open_digest(
            &mut handle,
            &DigestSpec {
                algorithm: DigestAlgorithm::Blake3,
                value: file.blake3.clone(),
            },
        )
        .map_err(|_| failure())?;
        handles.push(handle);
    }
    let files: Vec<_> = expected.into_values().collect();
    // The graph cannot launch an undeclared/unverified executable.
    for step in &snapshot.plan.steps {
        let executable = match step {
            super::plan::PlanStep::Exec(super::plan::ExecStep::StartRuntime { runtime }) => {
                &runtime.executable
            }
            super::plan::PlanStep::Exec(super::plan::ExecStep::StartProcess { process }) => {
                &process.executable
            }
            _ => continue,
        };
        if !files.iter().any(|(path, file)| {
            path.eq_ignore_ascii_case(executable) && file.role == CriticalFileRole::MainExecutable
        }) {
            return Err(FusionError::new(
                ErrorCode::CorruptedCriticalBinary,
                "A process has no verified main executable.",
            ));
        }
    }
    Ok((
        VerifiedCriticalFiles {
            _files: handles,
            _directories: directories,
        },
        files,
    ))
}

fn local_error(layer: &str, path: &str) -> FusionError {
    FusionError::new(
        ErrorCode::LocalBaseModified,
        "The located base changed; rebuild the workspace from the current installation.",
    )
    .with_detail("layerId", layer)
    .with_detail("path", path)
}
