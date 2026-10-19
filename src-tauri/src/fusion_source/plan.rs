//! Declarative Fetch -> Transform -> Env -> Exec planning. No step in this
//! module performs IO, resolves a release, writes a file, or starts a process.
//!
//! Serde here supports storage and inspection. Nested schema types retain
//! their wire contracts; dedicated IPC DTO conversion belongs to Stage 4.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::canonical::{CanonicalGameManifest, CasAccess, QualifiedGameId};
use super::capabilities::{
    unsupported_capability, CapabilityNegotiator, CapabilityRegistry, CapabilitySelection,
    NegotiationReport,
};
use super::errors::{ErrorCode, FusionError};
use super::evolution::SchemaUpcaster;
use super::schema::{
    ArchiveFormat, ConfigTemplateSpec, CriticalFileRole, DigestSpec, FallbackStrategy,
    GithubReleaseProvider, LayerKind, LayerSource, LocalPlacementPolicy, PatchFormat, PatchSpec,
    PersistenceSpec, PostSetupStep, ProcessSpec, ProjectionStrategy, RuntimeChannel, RuntimeSpec,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOptions {
    /// None uses defaults. Some is the exact set of optional addon/mod/cache
    /// layers requested by the user; dependencies never silently enable one.
    pub enabled_addons: Option<BTreeSet<String>>,
    pub runtime_channel: Option<RuntimeChannel>,
    /// Facts supplied by a future locator/executor, never inspected here.
    pub local_base_volume: Option<String>,
    pub library_volume: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPlan {
    pub game: QualifiedGameId,
    pub manifest_version: String,
    pub selected_layers: Vec<String>,
    pub runtime_channel: Option<RuntimeChannel>,
    pub negotiation: NegotiationReport,
    pub placement: WorkspacePlacement,
    pub steps: Vec<PlanStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", content = "operation", rename_all = "camelCase")]
pub enum PlanStep {
    Fetch(FetchStep),
    Transform(TransformStep),
    Env(EnvStep),
    Exec(ExecStep),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FetchStep {
    AcquireLayer {
        layer_id: String,
        source: LayerSource,
        transport_digest: Option<DigestSpec>,
    },
    ResolveGithubRelease {
        layer_id: String,
        provider: GithubReleaseProvider,
        required_lock: ProviderLockRequirement,
    },
}

/// A dynamic plan cannot execute until a specific release/asset, transport
/// digest, tool version and critical-file digests are pinned in its snapshot.
/// The pinned channel's digests must never attest an arbitrary latest release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLockRequirement {
    pub release_and_asset: bool,
    pub transport_digest: bool,
    pub critical_files: Vec<ProviderCriticalFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCriticalFile {
    pub path: String,
    pub role: CriticalFileRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TransformStep {
    VerifyTransport {
        layer_id: String,
        digest: DigestSpec,
    },
    ExtractLayer {
        layer_id: String,
        archive: ArchiveFormat,
        cas_access: CasAccess,
    },
    ProjectLayer {
        layer_id: String,
        mount_path: String,
        strategy: ProjectionStrategy,
        cas_access: CasAccess,
    },
    /// Replace the projected hardlink before making a writable physical copy.
    MaterializeWritablePaths {
        layer_id: String,
        paths: Vec<String>,
        strategy: ProjectionStrategy,
    },
    /// The output is always detached, even when input_path == output_path.
    ApplyPatch {
        layer_id: String,
        patch: PatchSpec,
        output_strategy: ProjectionStrategy,
    },
    MountOverlay {
        layer_ids: Vec<String>,
        cas_access: CasAccess,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EnvStep {
    PreserveSaves {
        persistence: PersistenceSpec,
        snapshot_before_layer_update: bool,
        preserve_originals_on_import: bool,
    },
    RenderConfigTemplate {
        template: ConfigTemplateSpec,
        output_strategy: ProjectionStrategy,
    },
    WasmTransform {
        id: String,
        layer_id: String,
        input_path: String,
        output_path: String,
        memory_buffer_only: bool,
        output_strategy: ProjectionStrategy,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExecStep {
    RequireManifestSync {
        game: QualifiedGameId,
    },
    CheckCriticalFile {
        file: PlannedCriticalFile,
    },
    /// Integrity attests bytes; execution additionally requires explicit trust
    /// for this exact digest. A provider lock never grants trust itself.
    RequireExecutableTrust {
        file: PlannedCriticalFile,
    },
    StartRuntime {
        runtime: RuntimeSpec,
    },
    StartProcess {
        process: ProcessSpec,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedCriticalFile {
    pub layer_id: String,
    pub workspace_path: String,
    pub role: CriticalFileRole,
    pub expected: CriticalDigestExpectation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CriticalDigestExpectation {
    DeclaredBlake3 { value: String },
    ProviderLock { layer_id: String, path: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceLocation {
    LibraryRoot,
    LocatedBaseVolume,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePlacement {
    pub location: WorkspaceLocation,
    pub cas_projection: ProjectionStrategy,
    pub local_projection: ProjectionStrategy,
    /// The executor must resolve unknown volume facts before using hardlinks.
    pub requires_volume_check: bool,
    /// Cross-volume fallback is explicit and never authorizes an implicit
    /// complete copy of a multi-gigabyte base installation.
    pub allow_full_base_copy: bool,
}

pub struct PlanBuilder;

impl PlanBuilder {
    pub fn build(
        manifest: &CanonicalGameManifest,
        options: &PlanOptions,
        registry: &CapabilityRegistry,
    ) -> Result<ExecutionPlan, FusionError> {
        // Public canonical values can be constructed or deserialized by callers.
        // Their type alone is not a substitute for path and graph validation.
        SchemaUpcaster::validate_canonical(manifest)?;
        let negotiation = CapabilityNegotiator::negotiate(manifest, registry);
        if let Some(blocker) = negotiation.blockers.first() {
            return Err(blocker.clone());
        }

        let runtime_channel = match (&manifest.runtime, options.runtime_channel) {
            (Some(runtime), channel) => Some(channel.unwrap_or(runtime.default_channel)),
            (None, None) => None,
            (None, Some(_)) => {
                return Err(game_error(
                    manifest,
                    ErrorCode::InvalidRuntimeChannel,
                    "A runtime channel was selected for a game without a runtime",
                ));
            }
        };
        let selected = select_layers(manifest, options, &negotiation, registry)?;
        let selected_layers: Vec<String> = manifest
            .layer_order
            .iter()
            .filter(|id| selected.contains(*id))
            .cloned()
            .collect();
        if selected_layers.len() != selected.len() {
            return Err(game_error(
                manifest,
                ErrorCode::InvalidManifest,
                "Canonical layer order does not contain every selected layer",
            ));
        }

        super::evolution::validate_selected_patch_integrity(manifest, &selected_layers)?;
        if let Some(runtime) = &manifest.runtime {
            if !selected.contains(&runtime.layer_id) {
                return Err(game_error(
                    manifest,
                    ErrorCode::MissingLayer,
                    "The selected runtime layer is disabled",
                )
                .with_detail("layerId", &runtime.layer_id));
            }
            let layer = &manifest.layers[&runtime.layer_id];
            match runtime_channel {
                Some(RuntimeChannel::Dynamic) if layer.dynamic_provider.is_none() => {
                    return Err(game_error(
                        manifest,
                        ErrorCode::InvalidRuntimeChannel,
                        "The runtime has no dynamic provider",
                    )
                    .with_detail("layerId", &runtime.layer_id));
                }
                Some(RuntimeChannel::Pinned)
                    if matches!(layer.source, LayerSource::LocalLocator { .. }) =>
                {
                    return Err(game_error(
                        manifest,
                        ErrorCode::InvalidRuntimeChannel,
                        "Pinned runtime requires a remote or user-provided artifact",
                    )
                    .with_detail("layerId", &runtime.layer_id));
                }
                _ => {}
            }
        }

        let has_local = selected_layers
            .iter()
            .any(|id| matches!(manifest.layers[id].source, LayerSource::LocalLocator { .. }));
        let placement = workspace_placement(manifest, options, has_local)?;
        let dynamic_layer = manifest.runtime.as_ref().and_then(|runtime| {
            (runtime_channel == Some(RuntimeChannel::Dynamic)).then_some(runtime.layer_id.as_str())
        });
        let overlay = negotiation.decisions.iter().any(|(capability, selection)| {
            capability.starts_with("vfs.overlay_mount.")
                && *selection == CapabilitySelection::Preferred
        });
        if !selected_layers.is_empty() {
            require_adapter(manifest, registry, "core.layers.v1")?;
            if !overlay {
                require_adapter(manifest, registry, "vfs.hardlink.v1")?;
            }
        }
        if manifest.orchestrator.is_some()
            || manifest.runtime.is_some()
            || manifest.launch.is_some()
        {
            require_adapter(manifest, registry, "proc.supervisor.v1")?;
        }
        if overlay
            && (placement.cas_projection == ProjectionStrategy::CrossVolumeFallback
                || placement.local_projection == ProjectionStrategy::CrossVolumeFallback)
        {
            return Err(game_error(
                manifest,
                ErrorCode::ProjectionUnavailable,
                "Overlay mounting cannot silently resolve a cross-volume projection",
            ));
        }

        let mut steps = Vec::new();
        for id in &selected_layers {
            let layer = &manifest.layers[id];
            if dynamic_layer == Some(id.as_str()) {
                require_adapter(manifest, registry, "provider.github_releases.v1")?;
                let provider = layer.dynamic_provider.clone().ok_or_else(|| {
                    game_error(
                        manifest,
                        ErrorCode::InvalidRuntimeChannel,
                        "Dynamic provider is missing",
                    )
                })?;
                steps.push(PlanStep::Fetch(FetchStep::ResolveGithubRelease {
                    layer_id: id.clone(),
                    provider,
                    required_lock: ProviderLockRequirement {
                        release_and_asset: true,
                        transport_digest: true,
                        critical_files: layer
                            .critical_files
                            .iter()
                            .map(|file| ProviderCriticalFile {
                                path: file.path.clone(),
                                role: file.role,
                            })
                            .collect(),
                    },
                }));
            } else {
                let source = match &layer.source {
                    LayerSource::Remote { mirrors, archive } => {
                        let supported: Vec<String> = mirrors
                            .iter()
                            .filter(|mirror| registry.supports(&mirror_capability(mirror)))
                            .cloned()
                            .collect();
                        if supported.is_empty() {
                            return Err(unsupported_capability(
                                manifest,
                                &mirror_capability(&mirrors[0]),
                                "No declared mirror has an implemented transport adapter",
                            )
                            .with_detail("layerId", id));
                        }
                        LayerSource::Remote {
                            mirrors: supported,
                            archive: *archive,
                        }
                    }
                    LayerSource::LocalLocator { .. } => {
                        require_adapter(manifest, registry, "local_locator.v1")?;
                        layer.source.clone()
                    }
                    LayerSource::UserProvided { .. } => layer.source.clone(),
                };
                steps.push(PlanStep::Fetch(FetchStep::AcquireLayer {
                    layer_id: id.clone(),
                    source,
                    transport_digest: layer.transport_digest.clone(),
                }));
            }
        }

        for id in &selected_layers {
            let layer = &manifest.layers[id];
            if dynamic_layer != Some(id.as_str()) {
                if let Some(digest) = &layer.transport_digest {
                    steps.push(PlanStep::Transform(TransformStep::VerifyTransport {
                        layer_id: id.clone(),
                        digest: digest.clone(),
                    }));
                }
            }
            if let LayerSource::Remote { archive, .. } | LayerSource::UserProvided { archive, .. } =
                layer.source
            {
                // Raw imports already are the payload. Archive imports share
                // the safe extraction boundary of remotely fetched bytes.
                if archive != ArchiveFormat::Raw {
                    steps.push(PlanStep::Transform(TransformStep::ExtractLayer {
                        layer_id: id.clone(),
                        archive,
                        cas_access: CasAccess::ReadOnly,
                    }));
                }
            }
            // A patch's acquired bytes are a tool input, never an overlay of
            // the workspace that could replace its own immutable input file.
            if let Some(patch) = &layer.patch {
                require_adapter(manifest, registry, patch_capability(patch.format))?;
                steps.push(PlanStep::Transform(TransformStep::ApplyPatch {
                    layer_id: id.clone(),
                    patch: patch.clone(),
                    output_strategy: ProjectionStrategy::CopyMutable,
                }));
            } else if overlay {
                // Add each read-only layer in dependency order. Private upper
                // writes from earlier patches/configs must survive later mounts.
                steps.push(PlanStep::Transform(TransformStep::MountOverlay {
                    layer_ids: vec![id.clone()],
                    cas_access: CasAccess::ReadOnly,
                }));
            } else {
                let strategy = if matches!(layer.source, LayerSource::LocalLocator { .. }) {
                    placement.local_projection
                } else {
                    placement.cas_projection
                };
                steps.push(PlanStep::Transform(TransformStep::ProjectLayer {
                    layer_id: id.clone(),
                    mount_path: layer.mount_path.clone(),
                    strategy,
                    cas_access: CasAccess::ReadOnly,
                }));
            }
            if !layer.writable_copy_paths.is_empty() {
                steps.push(PlanStep::Transform(
                    TransformStep::MaterializeWritablePaths {
                        layer_id: id.clone(),
                        paths: layer
                            .writable_copy_paths
                            .iter()
                            .map(|path| mounted_path(&layer.mount_path, path))
                            .collect(),
                        strategy: ProjectionStrategy::CopyMutable,
                    },
                ));
            }
        }
        append_environment(manifest, &selected, &negotiation, registry, &mut steps)?;
        let game = QualifiedGameId {
            source_id: manifest.source_id.clone(),
            game_id: manifest.id.clone(),
        };
        if manifest
            .orchestrator
            .as_ref()
            .is_some_and(|orchestrator| orchestrator.strict_sync)
        {
            steps.push(PlanStep::Exec(ExecStep::RequireManifestSync {
                game: game.clone(),
            }));
        }
        // Later layers override earlier critical paths, exactly as projection
        // does. A patched binary must not be checked against both old and new
        // digests. Local-locator keys participate in the same final inventory.
        let mut critical_files = BTreeMap::new();
        for id in &selected_layers {
            let layer = &manifest.layers[id];
            let locator_files = match &layer.source {
                LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
                _ => &[],
            };
            for file in locator_files.iter().chain(&layer.critical_files) {
                let workspace_path = mounted_path(&layer.mount_path, &file.path);
                let expected = if dynamic_layer == Some(id.as_str()) {
                    CriticalDigestExpectation::ProviderLock {
                        layer_id: id.clone(),
                        path: file.path.clone(),
                    }
                } else {
                    CriticalDigestExpectation::DeclaredBlake3 {
                        value: file.blake3.clone(),
                    }
                };
                critical_files.insert(
                    workspace_path.to_lowercase(),
                    PlannedCriticalFile {
                        layer_id: id.clone(),
                        workspace_path,
                        role: file.role,
                        expected,
                    },
                );
            }
        }
        let critical_files: Vec<_> = critical_files.into_values().collect();
        steps.extend(
            critical_files
                .iter()
                .cloned()
                .map(|file| PlanStep::Exec(ExecStep::CheckCriticalFile { file })),
        );
        steps.extend(
            critical_files
                .into_iter()
                .map(|file| PlanStep::Exec(ExecStep::RequireExecutableTrust { file })),
        );
        if let Some(orchestrator) = &manifest.orchestrator {
            for process in ordered_processes(manifest, &orchestrator.processes)? {
                steps.push(PlanStep::Exec(ExecStep::StartProcess { process }));
            }
        } else if let Some(process) = &manifest.launch {
            steps.push(PlanStep::Exec(ExecStep::StartProcess {
                process: process.clone(),
            }));
        } else if let Some(runtime) = &manifest.runtime {
            steps.push(PlanStep::Exec(ExecStep::StartRuntime {
                runtime: runtime.clone(),
            }));
        }

        Ok(ExecutionPlan {
            game,
            manifest_version: manifest.version.clone(),
            selected_layers,
            runtime_channel,
            negotiation,
            placement,
            steps,
        })
    }
}

fn select_layers(
    manifest: &CanonicalGameManifest,
    options: &PlanOptions,
    negotiation: &NegotiationReport,
    registry: &CapabilityRegistry,
) -> Result<BTreeSet<String>, FusionError> {
    if let Some(addons) = &options.enabled_addons {
        for id in addons {
            let valid = manifest.layers.get(id).is_some_and(|layer| {
                layer.optional
                    && matches!(
                        layer.kind,
                        LayerKind::Addon | LayerKind::Mod | LayerKind::ShaderCache
                    )
            });
            if !valid {
                return Err(game_error(
                    manifest,
                    ErrorCode::UnknownAddon,
                    "Selection must identify an optional addon, mod, or shader cache",
                )
                .with_detail("layerId", id));
            }
        }
    }
    let mut selected: BTreeSet<String> = manifest
        .layers
        .values()
        .filter(|layer| {
            !layer.optional
                || match (&options.enabled_addons, layer.kind) {
                    (Some(addons), LayerKind::Addon | LayerKind::Mod | LayerKind::ShaderCache) => {
                        addons.contains(&layer.id)
                    }
                    _ => layer.default_enabled,
                }
        })
        .map(|layer| layer.id.clone())
        .collect();

    // A missing optional patch adapter actually removes its optional patch,
    // rather than leaving an unexecutable operation in a supposedly valid plan.
    for layer in manifest.layers.values() {
        let capability = layer
            .patch
            .as_ref()
            .map(|patch| patch_capability(patch.format));
        if selected.contains(&layer.id)
            && capability.is_some_and(|capability| negotiation.omits(capability))
        {
            if !layer.optional {
                return Err(unsupported_capability(
                    manifest,
                    capability.unwrap_or_default(),
                    "An unavailable optional capability is consumed by a required layer",
                )
                .with_detail("layerId", &layer.id));
            }
            selected.remove(&layer.id);
        }
        if !selected.contains(&layer.id) || !layer.optional {
            continue;
        }
        let omitted_source = match &layer.source {
            LayerSource::Remote { mirrors, .. } => mirrors.iter().all(|mirror| {
                let capability = mirror_capability(mirror);
                !registry.supports(&capability) && negotiation.omits(&capability)
            }),
            LayerSource::LocalLocator { .. } => {
                !registry.supports("local_locator.v1") && negotiation.omits("local_locator.v1")
            }
            LayerSource::UserProvided { .. } => false,
        };
        if omitted_source {
            selected.remove(&layer.id);
        }
    }
    for id in &selected {
        for dependency in &manifest.layers[id].depends_on {
            if !selected.contains(dependency) {
                return Err(game_error(
                    manifest,
                    ErrorCode::MissingLayer,
                    "A selected layer depends on a disabled or unavailable layer",
                )
                .with_detail("layerId", id)
                .with_detail("dependency", dependency));
            }
        }
    }
    Ok(selected)
}

fn workspace_placement(
    manifest: &CanonicalGameManifest,
    options: &PlanOptions,
    has_local: bool,
) -> Result<WorkspacePlacement, FusionError> {
    let mut placement = WorkspacePlacement {
        location: WorkspaceLocation::LibraryRoot,
        cas_projection: ProjectionStrategy::Hardlink,
        local_projection: ProjectionStrategy::Hardlink,
        requires_volume_check: false,
        allow_full_base_copy: false,
    };
    if !has_local {
        return Ok(placement);
    }
    let same_volume = options
        .local_base_volume
        .as_ref()
        .filter(|volume| !volume.trim().is_empty())
        .zip(
            options
                .library_volume
                .as_ref()
                .filter(|volume| !volume.trim().is_empty()),
        )
        .map(|(base, library)| {
            base.trim_end_matches(['/', '\\'])
                .eq_ignore_ascii_case(library.trim_end_matches(['/', '\\']))
        });
    placement.requires_volume_check = same_volume.is_none();
    match manifest.local_placement {
        LocalPlacementPolicy::RequireSameVolume if same_volume == Some(false) => {
            return Err(game_error(
                manifest,
                ErrorCode::CrossVolumeLinkNotSupported,
                "The located base is on a different volume from library_root",
            )
            .with_detail("localBaseVolume", &options.local_base_volume)
            .with_detail("libraryVolume", &options.library_volume));
        }
        LocalPlacementPolicy::WorkspaceOnBaseVolume => {
            placement.location = WorkspaceLocation::LocatedBaseVolume;
            // Library CAS stays in library_root. Its links now cross volumes;
            // unknown facts require an explicit deferred fallback as well.
            if same_volume != Some(true) {
                placement.cas_projection = ProjectionStrategy::CrossVolumeFallback;
            }
        }
        LocalPlacementPolicy::CrossVolumeFallback => {
            if same_volume != Some(true) {
                placement.local_projection = ProjectionStrategy::CrossVolumeFallback;
            }
        }
        LocalPlacementPolicy::RequireSameVolume => {}
    }
    Ok(placement)
}

fn append_environment(
    manifest: &CanonicalGameManifest,
    selected: &BTreeSet<String>,
    negotiation: &NegotiationReport,
    registry: &CapabilityRegistry,
    steps: &mut Vec<PlanStep>,
) -> Result<(), FusionError> {
    if !manifest.persistence.saves_paths.is_empty() {
        if negotiation.omits("persistence.saves.v1") {
            return Err(unsupported_capability(
                manifest,
                "persistence.saves.v1",
                "Save protection cannot be silently omitted from a game that declares saves",
            ));
        }
        require_adapter(manifest, registry, "persistence.saves.v1")?;
        steps.push(PlanStep::Env(EnvStep::PreserveSaves {
            persistence: manifest.persistence.clone(),
            snapshot_before_layer_update: true,
            preserve_originals_on_import: true,
        }));
    }

    let fallback_template_ids: BTreeSet<&str> = manifest
        .post_setup_steps
        .iter()
        .filter_map(|step| match step {
            PostSetupStep::WasmTransform {
                fallback_template, ..
            } => fallback_template.as_deref(),
        })
        .collect();
    let mut render_ids = BTreeSet::new();
    for template in &manifest.config_templates {
        if !fallback_template_ids.contains(template.id.as_str()) {
            render_ids.insert(template.id.as_str());
        }
    }
    let mut wasm_steps = Vec::new();
    for step in &manifest.post_setup_steps {
        let PostSetupStep::WasmTransform {
            id,
            layer_id,
            input_path,
            output_path,
            optional,
            fallback_template,
        } = step;
        if !selected.contains(layer_id) {
            if *optional {
                continue;
            }
            return Err(game_error(
                manifest,
                ErrorCode::MissingLayer,
                "Required post-setup step refers to a disabled layer",
            )
            .with_detail("stepId", id)
            .with_detail("layerId", layer_id));
        }
        match negotiation.selection("ext.wasm_transform.v1") {
            Some(CapabilitySelection::Preferred) if registry.supports("ext.wasm_transform.v1") => {
                wasm_steps.push(PlanStep::Env(EnvStep::WasmTransform {
                    id: id.clone(),
                    layer_id: layer_id.clone(),
                    input_path: input_path.clone(),
                    output_path: output_path.clone(),
                    memory_buffer_only: true,
                    output_strategy: ProjectionStrategy::CopyMutable,
                }));
            }
            Some(CapabilitySelection::Fallback {
                strategy: FallbackStrategy::BuiltinConfigTemplates,
            }) => {
                let template_id = fallback_template.as_ref().ok_or_else(|| {
                    unsupported_capability(
                        manifest,
                        "ext.wasm_transform.v1",
                        "The selected template fallback has no target template",
                    )
                    .with_detail("stepId", id)
                })?;
                if !manifest
                    .config_templates
                    .iter()
                    .any(|template| template.id == *template_id)
                {
                    return Err(game_error(
                        manifest,
                        ErrorCode::InvalidManifest,
                        "The selected fallback template does not exist",
                    )
                    .with_detail("templateId", template_id));
                }
                render_ids.insert(template_id.as_str());
            }
            _ if *optional => {}
            _ => {
                return Err(unsupported_capability(
                    manifest,
                    "ext.wasm_transform.v1",
                    "Required Wasm transform has no supported implementation or fallback",
                )
                .with_detail("stepId", id));
            }
        }
    }
    if !render_ids.is_empty() && negotiation.omits("core.config_templates.v1") {
        return Err(unsupported_capability(
            manifest,
            "core.config_templates.v1",
            "Required config templates cannot be silently omitted",
        ));
    }
    if !render_ids.is_empty() {
        require_adapter(manifest, registry, "core.config_templates.v1")?;
    }
    for template in &manifest.config_templates {
        if render_ids.contains(template.id.as_str()) {
            steps.push(PlanStep::Env(EnvStep::RenderConfigTemplate {
                template: template.clone(),
                output_strategy: ProjectionStrategy::CopyMutable,
            }));
        }
    }
    steps.extend(wasm_steps);
    Ok(())
}

fn ordered_processes(
    manifest: &CanonicalGameManifest,
    processes: &[ProcessSpec],
) -> Result<Vec<ProcessSpec>, FusionError> {
    let mut pending: BTreeMap<&str, &ProcessSpec> = processes
        .iter()
        .map(|process| (process.id.as_str(), process))
        .collect();
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::new();
    while !pending.is_empty() {
        let next = pending.iter().find_map(|(id, process)| {
            process
                .depends_on
                .iter()
                .all(|dependency| emitted.contains(dependency.as_str()))
                .then_some(*id)
        });
        let Some(id) = next else {
            return Err(game_error(
                manifest,
                ErrorCode::InvalidManifest,
                "Process graph is cyclic or contains a missing dependency",
            ));
        };
        let Some(process) = pending.remove(id) else {
            return Err(game_error(
                manifest,
                ErrorCode::InvalidManifest,
                "Process graph changed while planning",
            ));
        };
        emitted.insert(id);
        ordered.push(process.clone());
    }
    Ok(ordered)
}

fn mounted_path(mount: &str, path: &str) -> String {
    if mount.is_empty() {
        path.to_owned()
    } else {
        format!("{mount}/{path}")
    }
}

fn game_error(manifest: &CanonicalGameManifest, code: ErrorCode, message: &str) -> FusionError {
    FusionError::new(code, message)
        .with_detail("sourceId", &manifest.source_id)
        .with_detail("gameId", &manifest.id)
}

fn require_adapter(
    manifest: &CanonicalGameManifest,
    registry: &CapabilityRegistry,
    capability: &str,
) -> Result<(), FusionError> {
    if registry.supports(capability) {
        Ok(())
    } else {
        Err(unsupported_capability(
            manifest,
            capability,
            "The planned operation has no implemented execution adapter",
        ))
    }
}

fn patch_capability(format: PatchFormat) -> &'static str {
    match format {
        PatchFormat::Xdelta3 => "patch.xdelta3.v1",
        PatchFormat::Bsdiff => "patch.bsdiff.v1",
    }
}

fn mirror_capability(mirror: &str) -> String {
    // The URI has already been validated; this only identifies an adapter.
    let scheme = mirror
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    format!("resolver.{scheme}.v1")
}
