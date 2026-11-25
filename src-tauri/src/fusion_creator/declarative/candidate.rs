//! Private candidate preparation shared by authoring and installation.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

use crate::fusion_creator::projection::{ProjectionLayer, ResolvedProjection};
use crate::fusion_source::cas::CasStore;
use crate::fusion_source::creator_schema::ComponentOrigin;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::program_schema::{
    ProgramComponent, ProgramControlsProfile, ProgramDescriptor, MAX_PROGRAM_BYTES,
};
use crate::fusion_source::CanonicalGameManifest;

use super::capabilities::{eden_param_package::EdenLocalSdlBinding, ProgramCapabilityRegistry};
use super::compiler::{compile_descriptor, compile_selected_profile, CompiledProgram};
use super::runtime::descriptor;
use super::state::ProgramPersistenceStore;

fn verify_component_input(
    manifest: &CanonicalGameManifest,
    program: &ProgramDescriptor,
    component: &ProgramComponent,
    inputs: &BTreeMap<String, LayerInput>,
    cas: &CasStore,
    check: &dyn Fn() -> Result<(), FusionError>,
) -> Result<ProjectionLayer, FusionError> {
    check()?;
    let Some(LayerInput::Cas {
        digest,
        runtime_lock: None,
    }) = inputs.get(&component.layer_id)
    else {
        return Err(invalid());
    };
    let object = cas.verify_checked(digest, check)?;
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
        check()?;
        if is_loadable(&file.path) {
            let projected =
                crate::fusion_source::evolution::mounted_path(&component.mount_path, &file.path);
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
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "The compiled manifest omits or contradicts an executable/loadable inventory record.",
                ));
            }
        }
    }
    Ok(ProjectionLayer {
        layer_id: component.layer_id.clone(),
        mount_path: component.mount_path.clone(),
        verified_inventory: object.files,
    })
}

/// Full normalized CAS inventories, never the descriptor's required-file subset.
pub fn verify_inputs(
    manifest: &CanonicalGameManifest,
    inputs: &BTreeMap<String, LayerInput>,
    cas: &CasStore,
) -> Result<CompiledProgram, FusionError> {
    let program = descriptor(manifest).ok_or_else(invalid)?;
    let mut inventories = Vec::with_capacity(program.components.len());
    for component in &program.components {
        inventories.push(verify_component_input(
            manifest,
            program,
            component,
            inputs,
            cas,
            &|| Ok(()),
        )?);
    }
    compile_descriptor(program, &inventories)
}

/// Verify CAS inventories for only the selected build profile's components with cancellation checks.
pub fn verify_selected_inputs_checked(
    manifest: &CanonicalGameManifest,
    build_profile_id: &str,
    inputs: &BTreeMap<String, LayerInput>,
    cas: &CasStore,
    check: &dyn Fn() -> Result<(), FusionError>,
) -> Result<ResolvedProjection, FusionError> {
    check()?;
    let program = descriptor(manifest).ok_or_else(invalid)?;
    let profile = program
        .build_profiles
        .iter()
        .find(|p| p.id == build_profile_id)
        .ok_or_else(invalid)?;
    let selected: BTreeSet<&str> = profile.layer_ids.iter().map(String::as_str).collect();
    if inputs.keys().any(|id| !selected.contains(id.as_str())) {
        return Err(invalid());
    }
    let mut inventories = Vec::with_capacity(selected.len());
    for component in &program.components {
        if selected.contains(component.layer_id.as_str()) {
            inventories.push(verify_component_input(
                manifest, program, component, inputs, cas, check,
            )?);
        }
    }
    compile_selected_profile(program, build_profile_id, &inventories)
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
    let build = program
        .build_profiles
        .iter()
        .find(|p| p.id == profile)
        .ok_or_else(invalid)?;
    prepare_with_controls(
        program,
        profile,
        build.controls_profile_ids.first().map(String::as_str),
        None,
        candidate,
        state,
    )
}

pub(crate) fn prepare_with_controls(
    program: &ProgramDescriptor,
    profile: &str,
    controls_profile_id: Option<&str>,
    local_sdl: Option<&EdenLocalSdlBinding>,
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
    let resolved_controls_id =
        controls_profile_id.or_else(|| build.controls_profile_ids.first().map(String::as_str));
    let controls = match resolved_controls_id {
        Some(id) => {
            if !build
                .controls_profile_ids
                .iter()
                .any(|allowed| allowed == id)
            {
                return Err(invalid());
            }
            Some(
                program
                    .controls_profiles
                    .iter()
                    .find(|item| item.id() == id)
                    .ok_or_else(invalid)?,
            )
        }
        None => None,
    };
    let registry = ProgramCapabilityRegistry;
    let mut controls_applied = false;
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
        let mut rendered =
            super::codecs::render_config(config, existing.as_deref(), &build.settings)?;
        if let Some(controls_profile) = controls {
            let targets_config = match controls_profile {
                ProgramControlsProfile::ApplicationDefaults { .. } => false,
                ProgramControlsProfile::EdenParamPackageV1 { config_id, .. } => {
                    config_id == &config.id
                }
            };
            if targets_config {
                let assignments =
                    registry.render_controls_plan(controls_profile, config, local_sdl)?;
                rendered = super::codecs::apply_ini_assignments(&rendered, assignments)?;
                controls_applied = true;
            }
        }
        fs_safety::atomic_write(&path, rendered.as_bytes())?;
        hashes.insert(
            config.path.clone(),
            blake3::hash(rendered.as_bytes()).to_hex().to_string(),
        );
    }
    if matches!(
        controls,
        Some(ProgramControlsProfile::EdenParamPackageV1 { .. })
    ) && !controls_applied
    {
        return Err(invalid());
    }
    Ok(hashes)
}

fn invalid() -> FusionError {
    FusionError::new(
        ErrorCode::InvalidManifest,
        "Invalid declarative candidate input.",
    )
}
