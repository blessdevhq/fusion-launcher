//! Private candidate preparation shared by authoring and installation.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use crate::fusion_creator::projection::ProjectionLayer;
use crate::fusion_source::cas::CasStore;
use crate::fusion_source::creator_schema::ComponentOrigin;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::program_schema::{ProgramDescriptor, MAX_PROGRAM_BYTES};
use crate::fusion_source::CanonicalGameManifest;

use super::compiler::{compile_descriptor, CompiledProgram};
use super::runtime::descriptor;
use super::state::ProgramPersistenceStore;

/// Full normalized CAS inventories, never the descriptor's required-file subset.
pub fn verify_inputs(
    manifest: &CanonicalGameManifest,
    inputs: &BTreeMap<String, LayerInput>,
    cas: &CasStore,
) -> Result<CompiledProgram, FusionError> {
    let program = descriptor(manifest).ok_or_else(invalid)?;
    let mut inventories = Vec::new();
    for component in &program.components {
        let Some(LayerInput::Cas {
            digest,
            runtime_lock: None,
        }) = inputs.get(&component.layer_id)
        else {
            return Err(invalid());
        };
        let object = cas.verify(digest)?;
        let expected = match &component.origin {
            ComponentOrigin::External {
                inventory_blake3, ..
            }
            | ComponentOrigin::UserProvided {
                inventory_blake3, ..
            } => Some(inventory_blake3),
            ComponentOrigin::Managed { .. } => None,
        };
        if expected.is_some_and(|expected| expected != &object.digest) {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Component inventory differs from its pinned origin.",
            ));
        }
        let layer = manifest
            .layers
            .get(&component.layer_id)
            .ok_or_else(invalid)?;
        for file in &object.files {
            if is_loadable(&file.path) {
                let projected = crate::fusion_source::evolution::mounted_path(
                    &component.mount_path,
                    &file.path,
                );
                let role = if projected == program.entrypoint.executable {
                    crate::fusion_source::schema::CriticalFileRole::MainExecutable
                } else {
                    crate::fusion_source::schema::CriticalFileRole::CoreLibrary
                };
                if !layer.critical_files.iter().any(|declared| {
                    declared.path == file.path
                        && declared.blake3 == file.blake3
                        && declared.role == role
                }) {
                    return Err(FusionError::new(ErrorCode::InvalidManifest,"The compiled manifest omits or contradicts an executable/loadable inventory record."));
                }
            }
        }
        inventories.push(ProjectionLayer {
            layer_id: component.layer_id.clone(),
            mount_path: component.mount_path.clone(),
            verified_inventory: object.files,
        });
    }
    compile_descriptor(program, &inventories)
}

pub fn is_loadable(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "exe" | "dll" | "ocx" | "cpl" | "drv" | "scr" | "pyd" | "so" | "dylib"
        )
    })
}

pub fn prepare(
    program: &ProgramDescriptor,
    profile: &str,
    candidate: &Path,
    state: &ProgramPersistenceStore,
) -> Result<BTreeMap<String, String>, FusionError> {
    let _candidate = fs_safety::guard(candidate)?;
    for directory in &program.directories {
        let path = if directory.is_empty() {
            candidate.to_owned()
        } else {
            candidate.join(fs_safety::validate_relative_path(directory)?)
        };
        fs_safety::ensure_directory(&path)?;
    }
    state.rehydrate(candidate, &program.persistence)?;
    let build = program
        .build_profiles
        .iter()
        .find(|p| p.id == profile)
        .ok_or_else(invalid)?;
    let mut hashes = BTreeMap::new();
    for config in &program.configs {
        let path = candidate.join(fs_safety::validate_relative_path(&config.path)?);
        let _parent = fs_safety::ensure_directory(path.parent().ok_or_else(invalid)?)?;
        let existing = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                fs_safety::check_metadata(&metadata, &path)?;
                let mut bytes = Vec::new();
                fs_safety::open_read(&path)?
                    .take(MAX_PROGRAM_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| fs_safety::io_error("Read private config", &path, e))?;
                if bytes.len() > MAX_PROGRAM_BYTES {
                    return Err(invalid());
                }
                Some(String::from_utf8(bytes).map_err(|_| invalid())?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(fs_safety::io_error("Inspect private config", &path, e)),
        };
        let rendered = super::codecs::render_config(config, existing.as_deref(), &build.settings)?;
        fs_safety::atomic_write(&path, rendered.as_bytes())?;
        hashes.insert(
            config.path.clone(),
            blake3::hash(rendered.as_bytes()).to_hex().to_string(),
        );
    }
    Ok(hashes)
}

fn invalid() -> FusionError {
    FusionError::new(
        ErrorCode::InvalidManifest,
        "Invalid declarative candidate input.",
    )
}
