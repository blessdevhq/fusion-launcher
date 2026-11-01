//! Pure finite-profile preflight. Validation does not authorize 1.1 execution.
use std::collections::{BTreeMap, BTreeSet};

use crate::fusion_source::canonical::CanonicalGameManifest;
use crate::fusion_source::creator_schema::{BuildProfile, CreatorManifest};
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::evolution::{mounted_path, validate_relative_path};
use crate::fusion_source::plan::PlanOptions;
use crate::fusion_source::schema::{CriticalFileRole, LayerSource};

use super::projection::{
    resolve_projection, ProjectionLayer, ProjectionRequest, ProjectionSemantics, ResolvedProjection,
};

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Result<BTreeSet<&'a str>, FusionError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.is_empty() || id.chars().count() > 128 || !seen.insert(id) {
            return Err(invalid(
                "Profile references must be nonempty, bounded and unique.",
            ));
        }
    }
    Ok(seen)
}

fn validate_metadata(manifest: &CanonicalGameManifest) -> Result<&CreatorManifest, FusionError> {
    let creator = manifest
        .creator
        .as_ref()
        .ok_or_else(|| invalid("Creator profiles are absent."))?;
    let creator = creator.as_legacy().ok_or_else(|| {
        FusionError::new(
            ErrorCode::ActionUnavailable,
            "Program profile compilation and execution are not registered.",
        )
    })?;
    if manifest.schema_version != "1.1"
        || creator.build_profiles.is_empty()
        || creator.controls_profiles.is_empty()
    {
        return Err(invalid(
            "Finite profiles require a 1.1 Creator manifest and declared profiles.",
        ));
    }
    unique_ids(
        creator
            .build_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    )?;
    let controls = unique_ids(
        creator
            .controls_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    )?;
    let components = unique_ids(
        creator
            .components
            .iter()
            .map(|component| component.layer_id.as_str()),
    )?;
    if components
        .iter()
        .any(|id| !manifest.layers.contains_key(*id))
    {
        return Err(invalid("A Creator component references an unknown layer."));
    }
    for profile in &creator.build_profiles {
        let layers = unique_ids(profile.layer_ids.iter().map(String::as_str))?;
        let pairings = unique_ids(profile.controls_profile_ids.iter().map(String::as_str))?;
        if layers.is_empty() || pairings.is_empty() || !pairings.is_subset(&controls) {
            return Err(invalid(
                "A build profile must select layers and known control profiles.",
            ));
        }
        for id in &layers {
            let layer = manifest
                .layers
                .get(*id)
                .ok_or_else(|| invalid("A profile references an unknown layer."))?;
            if !components.contains(id)
                || layer
                    .depends_on
                    .iter()
                    .any(|dependency| !layers.contains(dependency.as_str()))
            {
                return Err(invalid(
                    "A profile excludes a declared component or dependency.",
                ));
            }
        }
        if manifest
            .layers
            .values()
            .any(|layer| !layer.optional && !layers.contains(layer.id.as_str()))
        {
            return Err(invalid("A profile excludes a required layer."));
        }
        if manifest
            .runtime
            .as_ref()
            .is_some_and(|runtime| !layers.contains(runtime.layer_id.as_str()))
        {
            return Err(invalid("A profile excludes its runtime layer."));
        }
    }
    let mut rules = BTreeSet::new();
    for rule in &creator.projection_rules {
        validate_relative_path(&rule.path, false, false, "creator.projection_rules.path")?;
        if !rules.insert(rule.path.to_lowercase())
            || !manifest.layers.contains_key(&rule.winning_layer)
        {
            return Err(invalid(
                "Projection decisions must be unique and reference existing layers.",
            ));
        }
    }
    Ok(creator)
}

/// Validate metadata before the execution gate, without selecting defaults.
/// Legacy requests with no Creator extension retain their existing unavailable gate.
pub fn validate_profile_selection<'a>(
    manifest: &'a CanonicalGameManifest,
    options: &PlanOptions,
) -> Result<Option<&'a BuildProfile>, FusionError> {
    if manifest.creator.is_none() {
        return Ok(None);
    }
    let creator = validate_metadata(manifest)?;
    if options.enabled_addons.is_some() {
        return Err(invalid(
            "Finite profiles cannot be mixed with legacy addon overrides.",
        ));
    }
    match (&options.build_profile_id, &options.controls_profile_id) {
        (None, None) => Ok(None),
        (Some(build), Some(controls)) => {
            let profile = creator
                .build_profiles
                .iter()
                .find(|profile| &profile.id == build)
                .ok_or_else(|| invalid("Unknown build profile."))?;
            if !profile.controls_profile_ids.contains(controls) {
                return Err(invalid("The control profile is not an allowed pairing."));
            }
            Ok(Some(profile))
        }
        _ => Err(invalid(
            "Build and controls profile IDs must be selected together.",
        )),
    }
}

/// Resolve every declared profile against complete verified inventories. The
/// returned records, not dependency order, identify the actual binary provider.
/// Control pairings are all validated; they do not alter files before a writer runs.
pub fn resolve_declared_profiles(
    manifest: &CanonicalGameManifest,
    inventories: &[ProjectionLayer],
) -> Result<BTreeMap<String, ResolvedProjection>, FusionError> {
    let creator = validate_metadata(manifest)?;
    let mut available = BTreeMap::new();
    for inventory in inventories {
        let layer = manifest
            .layers
            .get(&inventory.layer_id)
            .ok_or_else(|| invalid("Inventory references an unknown layer."))?;
        if inventory.mount_path != layer.mount_path
            || available
                .insert(inventory.layer_id.as_str(), inventory)
                .is_some()
        {
            return Err(invalid(
                "Inventory has a duplicate layer or mismatched mount.",
            ));
        }
    }
    let mut projections = BTreeMap::new();
    for profile in &creator.build_profiles {
        let mut selected = Vec::new();
        for id in &profile.layer_ids {
            let layer = &manifest.layers[id];
            if layer.patch.is_some() {
                return Err(FusionError::new(
                    ErrorCode::ActionUnavailable,
                    "Explicit profile patch execution is unavailable.",
                ));
            }
            let inventory = available
                .get(id.as_str())
                .ok_or_else(|| invalid("A selected profile layer has no verified inventory."))?;
            selected.push((**inventory).clone());
        }
        let projection = resolve_projection(ProjectionRequest {
            semantics: ProjectionSemantics::Explicit11,
            selected_layers: &selected,
            rules: &creator.projection_rules,
        })?;
        validate_critical_projection(manifest, profile, &projection)?;
        projections.insert(profile.id.clone(), projection);
    }
    Ok(projections)
}

fn validate_critical_projection(
    manifest: &CanonicalGameManifest,
    profile: &BuildProfile,
    projection: &ResolvedProjection,
) -> Result<(), FusionError> {
    let mut critical_paths = BTreeSet::new();
    for id in &profile.layer_ids {
        let layer = &manifest.layers[id];
        let keys = match &layer.source {
            LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
            _ => &[],
        };
        for file in keys.iter().chain(&layer.critical_files) {
            critical_paths.insert(mounted_path(&layer.mount_path, &file.path).to_lowercase());
        }
    }
    for path in &critical_paths {
        let actual = projection
            .files
            .get(path)
            .ok_or_else(|| invalid("A profile omits a declared critical file."))?;
        let provider = &manifest.layers[&actual.winning_layer];
        let keys = match &provider.source {
            LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
            _ => &[],
        };
        let mut declarations = keys
            .iter()
            .chain(&provider.critical_files)
            .filter(|file| mounted_path(&provider.mount_path, &file.path).to_lowercase() == *path);
        let declaration = declarations
            .next()
            .ok_or_else(|| invalid("The actual critical-file provider has no declaration."))?;
        if declarations.any(|other| {
            other.role != declaration.role
                || !other.blake3.eq_ignore_ascii_case(&declaration.blake3)
        }) {
            return Err(invalid(
                "The actual provider has conflicting critical-file declarations.",
            ));
        }
        if !declaration.blake3.eq_ignore_ascii_case(&actual.blake3) {
            return Err(FusionError::new(
                ErrorCode::CorruptedCriticalBinary,
                "The actual provider does not match the critical-file digest.",
            ));
        }
    }
    let mut executables = Vec::new();
    if let Some(runtime) = &manifest.runtime {
        executables.push(runtime.executable.as_str());
    }
    if let Some(launch) = &manifest.launch {
        executables.push(launch.executable.as_str());
    }
    if let Some(orchestrator) = &manifest.orchestrator {
        executables.extend(
            orchestrator
                .processes
                .iter()
                .map(|process| process.executable.as_str()),
        );
    }
    for executable in executables {
        let key = executable.to_lowercase();
        if !projection.files.contains_key(&key) || !critical_paths.contains(&key) {
            return Err(invalid(
                "A profile omits a declared executable or its integrity declaration.",
            ));
        }
        let actual = &projection.files[&key];
        let provider = &manifest.layers[&actual.winning_layer];
        let keys = match &provider.source {
            LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
            _ => &[],
        };
        if !keys.iter().chain(&provider.critical_files).any(|file| {
            file.role == CriticalFileRole::MainExecutable
                && mounted_path(&provider.mount_path, &file.path).to_lowercase() == key
        }) {
            return Err(invalid(
                "The actual executable provider must declare main_executable.",
            ));
        }
    }
    let mut paths: BTreeSet<String> = projection.files.keys().cloned().collect();
    for template in &manifest.config_templates {
        let path = template.path.to_lowercase();
        validate_relative_path(&template.path, false, false, "config_templates.path")?;
        if critical_paths.contains(&path) {
            return Err(FusionError::new(
                ErrorCode::InvalidTemplate,
                "A config template cannot replace a critical binary.",
            ));
        }
        if paths.iter().any(|other| {
            other.starts_with(&(path.clone() + "/")) || path.starts_with(&(other.clone() + "/"))
        }) {
            return Err(FusionError::new(
                ErrorCode::InvalidTemplate,
                "Config output has a file/directory conflict.",
            ));
        }
        paths.insert(path);
    }
    Ok(())
}
