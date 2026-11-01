//! Process-local argument/environment preparation; never a shell or template engine.
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::{ProgramArgument, ProgramDescriptor};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentApproval {
    descriptor_blake3: String,
    names: BTreeSet<String>,
}

impl EnvironmentApproval {
    /// The caller records an explicit local decision for this exact descriptor.
    pub fn for_descriptor(
        program: &ProgramDescriptor,
        names: BTreeSet<String>,
    ) -> Result<Self, FusionError> {
        let value = serde_json::to_value(program).map_err(|_| invalid_approval())?;
        let bytes = serde_json::to_vec(&value).map_err(|_| invalid_approval())?;
        Ok(Self {
            descriptor_blake3: blake3::hash(&bytes).to_hex().to_string(),
            names,
        })
    }
}

pub fn approved_environment(
    program: &ProgramDescriptor,
    approval: Option<&EnvironmentApproval>,
) -> Result<BTreeSet<String>, FusionError> {
    let names: BTreeSet<_> = program
        .entrypoint
        .environment_paths
        .iter()
        .map(|p| p.name.clone())
        .collect();
    if names.is_empty() {
        return Ok(names);
    }
    let approval = approval.ok_or_else(invalid_approval)?;
    let expected = EnvironmentApproval::for_descriptor(program, names.clone())?;
    if approval.descriptor_blake3 != expected.descriptor_blake3 || approval.names != names {
        return Err(invalid_approval());
    }
    Ok(names)
}

fn invalid_approval() -> FusionError {
    FusionError::new(
        ErrorCode::ActionUnavailable,
        "Application environment paths require approval for this exact descriptor.",
    )
}

pub fn descriptor(
    manifest: &crate::fusion_source::CanonicalGameManifest,
) -> Option<&ProgramDescriptor> {
    manifest
        .creator
        .as_ref()?
        .as_program()
        .map(|p| &p.program_descriptor)
}

/// Compile only host-owned fields. The public descriptor remains the authority;
/// precompiled fields are never an alternative source of launch instructions.
pub fn execution_manifest(
    manifest: &crate::fusion_source::CanonicalGameManifest,
) -> Result<crate::fusion_source::CanonicalGameManifest, FusionError> {
    use crate::fusion_source::schema::{PersistenceSpec, ProcessRole, ProcessSpec};
    let program = descriptor(manifest).ok_or_else(|| {
        FusionError::new(ErrorCode::InvalidManifest, "Program descriptor is absent.")
    })?;
    super::compiler::validate_descriptor_metadata(program)?;
    let launch = ProcessSpec {
        id: "program".into(),
        executable: program.entrypoint.executable.clone(),
        args: program
            .entrypoint
            .arguments
            .iter()
            .map(|arg| match arg {
                ProgramArgument::Literal { value } => value.clone(),
                ProgramArgument::WorkspacePath { path, prefix } => {
                    format!("{prefix}{{{{WORKSPACE_DIR}}}}/{path}")
                }
            })
            .collect(),
        env: program
            .entrypoint
            .environment_paths
            .iter()
            .map(|v| (v.name.clone(), format!("{{{{WORKSPACE_DIR}}}}/{}", v.path)))
            .collect(),
        depends_on: Vec::new(),
        role: ProcessRole::Primary,
    };
    let persistence = PersistenceSpec {
        saves_paths: program
            .persistence
            .iter()
            .map(|p| format!("workspace/{}", p.path))
            .collect(),
    };
    if manifest.runtime.is_some()
        || manifest.orchestrator.is_some()
        || manifest
            .launch
            .as_ref()
            .is_some_and(|value| value != &launch)
        || (!manifest.persistence.saves_paths.is_empty() && manifest.persistence != persistence)
        || !manifest.config_templates.is_empty()
        || !manifest.post_setup_steps.is_empty()
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Legacy execution fields disagree with the program descriptor.",
        ));
    }
    let component_ids: BTreeSet<_> = program
        .components
        .iter()
        .map(|c| c.layer_id.as_str())
        .collect();
    if component_ids.len() != manifest.layers.len()
        || manifest
            .layers
            .keys()
            .any(|id| !component_ids.contains(id.as_str()))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Manifest layers disagree with descriptor components.",
        ));
    }
    for component in &program.components {
        let layer = &manifest.layers[&component.layer_id];
        if layer.mount_path != component.mount_path
            || layer.patch.is_some()
            || layer.dynamic_provider.is_some()
            || !layer.writable_copy_paths.is_empty()
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Layer layout disagrees with descriptor components.",
            ));
        }
    }
    let mut compiled = manifest.clone();
    compiled.launch = Some(launch);
    compiled.persistence = persistence;
    Ok(compiled)
}

/// Lower a validated finite selection to the existing planner's operations.
/// The signed descriptor is retained by the workspace, never replaced or migrated.
pub fn build_program_plan(
    manifest: &crate::fusion_source::CanonicalGameManifest,
    options: &crate::fusion_source::PlanOptions,
    registry: &crate::fusion_source::CapabilityRegistry,
) -> Result<crate::fusion_source::ExecutionPlan, FusionError> {
    use crate::fusion_source::{PlanBuilder, PlanOptions};
    let program = descriptor(manifest).ok_or_else(|| {
        FusionError::new(ErrorCode::InvalidManifest, "Program descriptor is absent.")
    })?;
    let (Some(build), Some(controls)) = (&options.build_profile_id, &options.controls_profile_id)
    else {
        return Err(FusionError::new(
            ErrorCode::ActionUnavailable,
            "A program requires an explicit build/control pair.",
        ));
    };
    if options.enabled_addons.is_some() || options.runtime_channel.is_some() {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Finite profiles cannot mix legacy addon or channel overrides.",
        ));
    }
    let profile = program
        .build_profiles
        .iter()
        .find(|p| &p.id == build)
        .ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidManifest, "Unknown program build profile.")
        })?;
    if !profile.controls_profile_ids.contains(controls) {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "The controls profile is not an allowed pairing.",
        ));
    }
    let control = program
        .controls_profiles
        .iter()
        .find(|p| p.id() == controls)
        .ok_or_else(|| FusionError::new(ErrorCode::InvalidManifest, "Unknown controls profile."))?;
    if !matches!(
        control,
        crate::fusion_source::program_schema::ProgramControlsProfile::ApplicationDefaults { .. }
    ) {
        return Err(FusionError::new(
            ErrorCode::ActionUnavailable,
            "Selected host capability is not registered.",
        ));
    }
    let mut lowered = execution_manifest(manifest)?;
    lowered.creator = None;
    lowered.schema_version = "1.0".into();
    let selected: BTreeSet<_> = profile.layer_ids.iter().cloned().collect();
    lowered.layers.retain(|id, _| selected.contains(id));
    lowered.layer_order.retain(|id| selected.contains(id));
    let addons = lowered
        .layers
        .values()
        .filter(|layer| layer.optional)
        .map(|layer| layer.id.clone())
        .collect();
    let mut plan = PlanBuilder::build(
        &lowered,
        &PlanOptions {
            enabled_addons: Some(addons),
            local_base_volume: options.local_base_volume.clone(),
            library_volume: options.library_volume.clone(),
            ..PlanOptions::default()
        },
        registry,
    )?;
    plan.build_profile_id = Some(build.clone());
    plan.controls_profile_id = Some(controls.clone());
    Ok(plan)
}

pub fn prepare_program_command(
    descriptor: &ProgramDescriptor,
    workspace: &Path,
    isolated: &BTreeMap<String, String>,
    approved: &BTreeSet<String>,
) -> Result<Command, FusionError> {
    super::compiler::validate_descriptor_metadata(descriptor)?;
    for variable in &descriptor.entrypoint.environment_paths {
        if !approved.contains(&variable.name) {
            return Err(FusionError::new(
                ErrorCode::ActionUnavailable,
                "Application environment paths require explicit local approval.",
            )
            .with_detail("variable", &variable.name));
        }
    }
    let _workspace = crate::fusion_source::fs_safety::guard(workspace)?;
    let relative = |path: &str| -> Result<std::path::PathBuf, FusionError> {
        if path.is_empty() {
            Ok(workspace.to_owned())
        } else {
            Ok(
                workspace.join(crate::fusion_source::fs_safety::validate_relative_path(
                    path,
                )?),
            )
        }
    };
    let cwd = relative(&descriptor.entrypoint.working_directory)?;
    let _cwd = crate::fusion_source::fs_safety::guard(&cwd)?;
    let mut command = Command::new(relative(&descriptor.entrypoint.executable)?);
    command.current_dir(cwd).env_clear();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command
            .env("SystemRoot", &root)
            .env("WINDIR", &root)
            .env("PATH", std::path::PathBuf::from(root).join("System32"));
    }
    for argument in &descriptor.entrypoint.arguments {
        match argument {
            ProgramArgument::Literal { value } => {
                command.arg(value);
            }
            ProgramArgument::WorkspacePath { path, prefix } => {
                let path = relative(path)?;
                command.arg(format!(
                    "{prefix}{}",
                    path.to_str().ok_or_else(|| FusionError::new(
                        ErrorCode::InvalidPath,
                        "Workspace path is not Unicode."
                    ))?
                ));
            }
        }
    }
    for variable in &descriptor.entrypoint.environment_paths {
        command.env(&variable.name, relative(&variable.path)?);
    }
    for (name, value) in isolated {
        command.env(name, value);
    }
    Ok(command)
}
