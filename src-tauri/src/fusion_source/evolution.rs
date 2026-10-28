//! Pure v1.0 parsing, inheritance resolution and validation.
//!
//! Hashes here describe expected bytes; parsing does not authenticate a source
//! or verify an artifact. Fetch adapters must verify transport bytes separately.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use base64::Engine;
use serde_json::Value;
use url::Url;

use super::canonical::{
    CanonicalGameManifest, CanonicalLayer, CanonicalSourceIndex, QualifiedGameId,
};
use super::errors::{ErrorCode, FusionError};
use super::schema::*;

const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_INHERITANCE_DEPTH: usize = 64;
const MAX_LAYERS: usize = 4096;

pub struct SchemaUpcaster;

impl SchemaUpcaster {
    /// Public canonical values may be deserialized from future storage adapters.
    /// Recheck their paths, identities and graph ordering at the planning boundary.
    pub fn validate_canonical(manifest: &CanonicalGameManifest) -> Result<(), FusionError> {
        validate_id(&manifest.source_id, "source_id")?;
        if manifest.schema_version != SCHEMA_VERSION {
            return Err(FusionError::new(
                ErrorCode::UnsupportedSchema,
                "Unsupported canonical schema version.",
            ));
        }
        if let Some(parent) = &manifest.extends {
            validate_id(&parent.source_id, "extends.source_id")?;
            validate_id(&parent.game_id, "extends.game_id")?;
            if parent.source_id == manifest.source_id && parent.game_id == manifest.id {
                return Err(FusionError::new(
                    ErrorCode::ExtendsCycle,
                    "A manifest cannot extend itself.",
                ));
            }
        }
        let mut seen = BTreeSet::new();
        let mut layers = Vec::with_capacity(manifest.layer_order.len());
        for id in &manifest.layer_order {
            unique(&mut seen, id, "layer_order")?;
            let layer = manifest.layers.get(id).ok_or_else(|| {
                FusionError::new(
                    ErrorCode::MissingLayer,
                    "Canonical layer order references an unknown layer.",
                )
                .with_detail("layerId", id)
            })?;
            if layer.id != *id {
                return Err(FusionError::new(
                    ErrorCode::InvalidId,
                    "Canonical layer ID differs from its map key.",
                )
                .with_detail("layerId", id));
            }
            if layer
                .patch
                .as_ref()
                .is_some_and(|patch| !layer.depends_on.contains(&patch.base_layer))
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Canonical patch dependency is missing.",
                ));
            }
            layers.push(LayerSpec::from(layer.clone()));
        }
        if seen.len() != manifest.layers.len() {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Canonical layer order omits layers.",
            ));
        }
        let wire = GameManifest {
            schema_version: manifest.schema_version.clone(),
            id: manifest.id.clone(),
            title: manifest.title.clone(),
            version: manifest.version.clone(),
            platform: manifest.platform.clone(),
            cover_url: manifest.cover_url.clone(),
            extends: manifest.extends.as_ref().map(ToString::to_string),
            capabilities: manifest.capabilities.clone(),
            fallback: manifest.fallback.clone(),
            layers,
            runtime: manifest.runtime.clone(),
            launch: manifest.launch.clone(),
            orchestrator: manifest.orchestrator.clone(),
            persistence: manifest.persistence.clone(),
            config_templates: manifest.config_templates.clone(),
            post_setup_steps: manifest.post_setup_steps.clone(),
            local_placement: manifest.local_placement,
        };
        validate_game_shape(&wire)?;
        if validate_resolved_manifest(&wire)? != manifest.layer_order {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Canonical layer order is not dependency-first.",
            ));
        }
        Ok(())
    }

    /// Parses a wire manifest. Parent-dependent validation happens during upcast.
    pub fn parse_game_manifest(json: &str) -> Result<GameManifest, FusionError> {
        parse_versioned_json(json)?;
        let manifest: GameManifest = serde_json::from_str(json)
            .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?;
        validate_game_shape(&manifest)?;
        Ok(manifest)
    }

    pub fn upcast_source_index(json: &str) -> Result<CanonicalSourceIndex, FusionError> {
        parse_versioned_json(json)?;
        let mut index: SourceIndex = serde_json::from_str(json)
            .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?;
        validate_id(&index.source_info.id, "source_info.id")?;
        nonempty(&index.source_info.name, "source_info.name")?;
        if let Some(endpoint) = &index.source_info.telemetry_endpoint {
            validate_https(endpoint, "source_info.telemetry_endpoint")?;
        }
        if let Some(key) = &index.source_info.public_key {
            let decoded = base64::engine::general_purpose::STANDARD.decode(key);
            if !matches!(decoded, Ok(ref bytes) if bytes.len() == 32) {
                return Err(field_error(
                    ErrorCode::InvalidPublicKey,
                    "source_info.public_key",
                    "Expected a base64-encoded 32-byte Ed25519 public key.",
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for game in &mut index.games {
            validate_id(&game.id, "games.id")?;
            // These IDs become directory components on Windows. Preserve the
            // publisher's spelling, but reserve its ASCII case-insensitive name.
            unique(&mut ids, &game.id.to_ascii_lowercase(), "games.id")?;
            nonempty(&game.title, "games.title")?;
            nonempty(&game.version, "games.version")?;
            if let Some(platform) = &game.platform {
                validate_id(platform, "games.platform")?;
            }
            if let Some(cover_url) = &game.cover_url {
                validate_https(cover_url, "games.cover_url")?;
            }
            if game.manifest_url.contains(':') || game.manifest_url.starts_with("//") {
                validate_https(&game.manifest_url, "games.manifest_url")?;
            } else {
                validate_relative_path(&game.manifest_url, false, false, "games.manifest_url")?;
            }
            validate_hash(&game.manifest_blake3, "games.manifest_blake3")?;
            game.manifest_blake3.make_ascii_lowercase();
        }
        Ok(CanonicalSourceIndex {
            schema_version: index.schema_version,
            source_info: index.source_info,
            games: index.games,
        })
    }

    /// Resolves the complete ancestry before building an executable canonical
    /// model. Missing parents are never treated as a partially runnable game.
    pub fn upcast_game_manifest(
        source_id: &str,
        json: &str,
        parents: &BTreeMap<QualifiedGameId, GameManifest>,
    ) -> Result<CanonicalGameManifest, FusionError> {
        validate_id(source_id, "source_id")?;
        let root = Self::parse_game_manifest(json)?;
        let key = QualifiedGameId {
            source_id: source_id.to_owned(),
            game_id: root.id.clone(),
        };
        let extends = root
            .extends
            .as_deref()
            .map(|reference| qualified_id(source_id, reference))
            .transpose()?;
        let mut stack = Vec::new();
        let mut manifest = resolve_inheritance(&key, &root, parents, &mut stack)?;
        normalize_manifest(&mut manifest);
        validate_game_shape(&manifest)?;
        let layer_order = validate_resolved_manifest(&manifest)?;
        let layers = manifest
            .layers
            .into_iter()
            .map(|layer| (layer.id.clone(), CanonicalLayer::from(layer)))
            .collect();
        Ok(CanonicalGameManifest {
            schema_version: manifest.schema_version,
            source_id: source_id.to_owned(),
            id: manifest.id,
            title: manifest.title,
            version: manifest.version,
            platform: manifest.platform,
            cover_url: manifest.cover_url,
            extends,
            capabilities: manifest.capabilities,
            fallback: manifest.fallback,
            layers,
            layer_order,
            runtime: manifest.runtime,
            launch: manifest.launch,
            orchestrator: manifest.orchestrator,
            persistence: manifest.persistence,
            config_templates: manifest.config_templates,
            post_setup_steps: manifest.post_setup_steps,
            local_placement: manifest.local_placement,
        })
    }
}

fn parse_versioned_json(json: &str) -> Result<Value, FusionError> {
    if json.len() > MAX_JSON_BYTES {
        return Err(FusionError::new(
            ErrorCode::InvalidJson,
            "Manifest exceeds the metadata size limit.",
        ));
    }
    let value: Value = serde_json::from_str(json)
        .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?;
    let version = value.get("schema_version").and_then(Value::as_str);
    // v1.0 is the first wire version. The pure normalization pass below is its
    // migration to the canonical model; future versions get explicit branches.
    if version != Some("1.0") {
        return Err(FusionError::new(
            ErrorCode::UnsupportedSchema,
            "Unsupported Fusion schema version.",
        )
        .with_detail("schemaVersion", value.get("schema_version")));
    }
    Ok(value)
}

fn resolve_inheritance(
    key: &QualifiedGameId,
    manifest: &GameManifest,
    parents: &BTreeMap<QualifiedGameId, GameManifest>,
    stack: &mut Vec<QualifiedGameId>,
) -> Result<GameManifest, FusionError> {
    if let Some(start) = stack.iter().position(|id| id == key) {
        let mut cycle: Vec<String> = stack[start..].iter().map(ToString::to_string).collect();
        cycle.push(key.to_string());
        return Err(
            FusionError::new(ErrorCode::ExtendsCycle, "Cyclic manifest inheritance.")
                .with_detail("cycle", cycle),
        );
    }
    if stack.len() >= MAX_INHERITANCE_DEPTH {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Manifest inheritance is too deep.",
        ));
    }
    validate_id(&key.source_id, "source_id")?;
    if manifest.id != key.game_id {
        return Err(FusionError::new(
            ErrorCode::InvalidId,
            "Parent identity does not match its lookup key.",
        )
        .with_detail("game", key.to_string()));
    }
    if manifest.schema_version != "1.0" {
        return Err(FusionError::new(
            ErrorCode::UnsupportedSchema,
            "Unsupported parent schema version.",
        ));
    }
    validate_game_shape(manifest)?;
    stack.push(key.clone());
    let result = if let Some(reference) = &manifest.extends {
        let parent_key = qualified_id(&key.source_id, reference)?;
        // A root present in the ancestry is a cycle even when it is not repeated
        // in the caller's parent map.
        if stack.contains(&parent_key) {
            let mut cycle: Vec<String> = stack.iter().map(ToString::to_string).collect();
            cycle.push(parent_key.to_string());
            return Err(
                FusionError::new(ErrorCode::ExtendsCycle, "Cyclic manifest inheritance.")
                    .with_detail("cycle", cycle),
            );
        }
        let parent = parents.get(&parent_key).ok_or_else(|| {
            FusionError::new(
                ErrorCode::MissingParent,
                "The base manifest must be supplied before upcasting.",
            )
            .with_detail("parent", parent_key.to_string())
        })?;
        let base = resolve_inheritance(&parent_key, parent, parents, stack)?;
        merge_manifest(base, manifest.clone())?
    } else {
        manifest.clone()
    };
    stack.pop();
    Ok(result)
}

fn merge_manifest(
    mut base: GameManifest,
    child: GameManifest,
) -> Result<GameManifest, FusionError> {
    let mut ids: BTreeSet<String> = base.layers.iter().map(|layer| layer.id.clone()).collect();
    for layer in &child.layers {
        if !matches!(
            layer.kind,
            LayerKind::Addon | LayerKind::Mod | LayerKind::Patch | LayerKind::ShaderCache
        ) {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "An extending manifest may add only addon, mod, patch or shader-cache layers.",
            )
            .with_detail("layer_id", &layer.id));
        }
        unique(&mut ids, &layer.id, "layers.id")?;
    }
    base.layers.extend(child.layers);
    if child.runtime.is_some() || child.launch.is_some() || child.orchestrator.is_some() {
        base.runtime = child.runtime;
        base.launch = child.launch;
        base.orchestrator = child.orchestrator;
    }
    base.capabilities
        .required
        .extend(child.capabilities.required);
    base.capabilities
        .optional
        .extend(child.capabilities.optional);
    base.capabilities.required.sort();
    base.capabilities.required.dedup();
    base.capabilities.optional.sort();
    base.capabilities.optional.dedup();
    base.capabilities
        .optional
        .retain(|capability| !base.capabilities.required.contains(capability));
    for fallback in child.fallback {
        base.fallback
            .retain(|old| old.capability != fallback.capability);
        base.fallback.push(fallback);
    }
    for template in child.config_templates {
        base.config_templates.retain(|old| old.id != template.id);
        base.config_templates.push(template);
    }
    base.post_setup_steps.extend(child.post_setup_steps);
    base.persistence
        .saves_paths
        .extend(child.persistence.saves_paths);
    base.persistence.saves_paths.sort();
    base.persistence.saves_paths.dedup();
    base.schema_version = child.schema_version;
    base.id = child.id;
    base.title = child.title;
    base.version = child.version;
    base.platform = child.platform.or(base.platform);
    base.cover_url = child.cover_url.or(base.cover_url);
    base.extends = child.extends;
    // Placement belongs to the derived installation, never to the locator's
    // publisher. Its default remains fail-closed on a cross-volume projection.
    base.local_placement = child.local_placement;
    Ok(base)
}

fn normalize_manifest(manifest: &mut GameManifest) {
    for layer in &mut manifest.layers {
        if let Some(digest) = &mut layer.transport_digest {
            digest.value.make_ascii_lowercase();
        }
        for file in &mut layer.critical_files {
            file.blake3.make_ascii_lowercase();
        }
        if let LayerSource::LocalLocator { locator } = &mut layer.source {
            for file in &mut locator.key_files {
                file.blake3.make_ascii_lowercase();
            }
        }
        if let Some(patch) = &mut layer.patch {
            patch.input_digest.value.make_ascii_lowercase();
            patch.output_digest.value.make_ascii_lowercase();
            if !layer.depends_on.contains(&patch.base_layer) {
                layer.depends_on.push(patch.base_layer.clone());
            }
        }
    }
}

fn validate_game_shape(manifest: &GameManifest) -> Result<(), FusionError> {
    validate_id(&manifest.id, "id")?;
    nonempty(&manifest.title, "title")?;
    nonempty(&manifest.version, "version")?;
    if let Some(cover_url) = &manifest.cover_url {
        validate_https(cover_url, "cover_url")?;
    }
    if let Some(platform) = &manifest.platform {
        validate_id(platform, "platform")?;
    }
    if let Some(reference) = &manifest.extends {
        qualified_id("current", reference)?;
    }
    if manifest.layers.len() > MAX_LAYERS {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Too many layers.",
        ));
    }
    validate_capabilities(&manifest.capabilities, &manifest.fallback)?;
    let mut layer_ids = BTreeSet::new();
    for layer in &manifest.layers {
        validate_id(&layer.id, "layers.id")?;
        unique(&mut layer_ids, &layer.id, "layers.id")?;
        validate_layer(layer)?;
    }
    let mut template_ids = BTreeSet::new();
    for template in &manifest.config_templates {
        validate_id(&template.id, "config_templates.id")?;
        unique(&mut template_ids, &template.id, "config_templates.id")?;
        validate_relative_path(&template.path, false, false, "config_templates.path")?;
    }
    for path in &manifest.persistence.saves_paths {
        let relative = path
            .strip_prefix("workspace/")
            .or_else(|| path.strip_prefix("{{APPDATA}}/"))
            .or_else(|| path.strip_prefix("{{LOCALAPPDATA}}/"))
            .ok_or_else(|| {
                field_error(
                    ErrorCode::InvalidPath,
                    "persistence.saves_paths",
                    "Saves must be rooted in workspace/, {{APPDATA}}/ or {{LOCALAPPDATA}}/.",
                )
            })?;
        validate_relative_path(relative, false, false, "persistence.saves_paths")?;
    }
    if let Some(runtime) = &manifest.runtime {
        validate_id(&runtime.layer_id, "runtime.layer_id")?;
        validate_relative_path(&runtime.executable, false, false, "runtime.executable")?;
        validate_arguments(&runtime.args)?;
        validate_environment(&runtime.env)?;
    }
    if let Some(process) = &manifest.launch {
        validate_process(process)?;
    }
    if let Some(orchestrator) = &manifest.orchestrator {
        for process in &orchestrator.processes {
            validate_process(process)?;
        }
    }
    let mut step_ids = BTreeSet::new();
    for step in &manifest.post_setup_steps {
        let PostSetupStep::WasmTransform {
            id,
            layer_id,
            input_path,
            output_path,
            fallback_template,
            ..
        } = step;
        validate_id(id, "post_setup_steps.id")?;
        unique(&mut step_ids, id, "post_setup_steps.id")?;
        validate_id(layer_id, "post_setup_steps.layer_id")?;
        validate_relative_path(input_path, false, false, "post_setup_steps.input_path")?;
        validate_relative_path(output_path, false, false, "post_setup_steps.output_path")?;
        if let Some(template) = fallback_template {
            validate_id(template, "post_setup_steps.fallback_template")?;
        }
    }
    Ok(())
}

fn validate_layer(layer: &LayerSpec) -> Result<(), FusionError> {
    if let Some(title) = &layer.title {
        nonempty(title, "layers.title")?;
    }
    validate_relative_path(&layer.mount_path, false, true, "layers.mount_path")?;
    let mut dependencies = BTreeSet::new();
    for dependency in &layer.depends_on {
        validate_id(dependency, "layers.depends_on")?;
        unique(&mut dependencies, dependency, "layers.depends_on")?;
    }
    for path in &layer.writable_copy_paths {
        validate_relative_path(path, true, false, "layers.writable_copy_paths")?;
        glob::Pattern::new(path).map_err(|e| {
            field_error(
                ErrorCode::InvalidPath,
                "layers.writable_copy_paths",
                &e.to_string(),
            )
        })?;
    }
    validate_critical_files(&layer.critical_files, "layers.critical_files")?;
    if let Some(digest) = &layer.transport_digest {
        validate_digest(digest, "layers.transport_digest")?;
    }
    if layer.optional
        && !matches!(
            layer.kind,
            LayerKind::Addon | LayerKind::Mod | LayerKind::ShaderCache
        )
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Only addon, mod and shader-cache layers may be optional.",
        )
        .with_detail("layer_id", &layer.id));
    }
    match &layer.source {
        LayerSource::Remote { mirrors, .. } => {
            if mirrors.is_empty() || layer.transport_digest.is_none() {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Remote layers require mirrors and a transport digest.",
                )
                .with_detail("layer_id", &layer.id));
            }
            for mirror in mirrors {
                validate_mirror(mirror)?;
            }
        }
        LayerSource::UserProvided { .. } => {}
        LayerSource::LocalLocator { locator } => {
            if layer.kind != LayerKind::Base {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Only a base layer may use local_locator.",
                ));
            }
            if locator.app_id.as_deref().is_none_or(str::is_empty) && locator.path_hints.is_empty()
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "A locator needs an app ID or path hint.",
                ));
            }
            if let Some(app_id) = &locator.app_id {
                nonempty(app_id, "local_locator.app_id")?;
            }
            if locator.key_files.is_empty() {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "A locator must verify key files.",
                ));
            }
            validate_critical_files(&locator.key_files, "local_locator.key_files")?;
            for path in &locator.path_hints {
                validate_locator_hint(path)?;
            }
        }
    }
    if matches!(
        layer.content,
        LayerContent::Bios | LayerContent::Firmware | LayerContent::Keys
    ) && !matches!(layer.source, LayerSource::UserProvided { .. })
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "BIOS, firmware and keys must be user-provided.",
        ));
    }
    if let Some(provider) = &layer.dynamic_provider {
        if layer.kind != LayerKind::Runtime {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Only runtime layers may use a dynamic provider.",
            ));
        }
        let parts: Vec<_> = provider.repo.split('/').collect();
        if parts.len() != 2
            || parts
                .iter()
                .any(|part| validate_id(part, "dynamic_provider.repo").is_err())
        {
            return Err(field_error(
                ErrorCode::InvalidManifest,
                "dynamic_provider.repo",
                "Expected owner/repository.",
            ));
        }
        validate_relative_path(
            &provider.asset_mask,
            true,
            false,
            "dynamic_provider.asset_mask",
        )?;
        if provider.asset_mask.contains('/') {
            return Err(field_error(
                ErrorCode::InvalidPath,
                "dynamic_provider.asset_mask",
                "Asset masks match filenames only.",
            ));
        }
        glob::Pattern::new(&provider.asset_mask).map_err(|e| {
            field_error(
                ErrorCode::InvalidPath,
                "dynamic_provider.asset_mask",
                &e.to_string(),
            )
        })?;
        match provider.channel {
            ReleaseChannel::Nightly => {
                let tag = provider.tag.as_deref().ok_or_else(|| {
                    field_error(
                        ErrorCode::InvalidManifest,
                        "dynamic_provider.tag",
                        "Nightly requires an explicit release tag.",
                    )
                })?;
                validate_relative_path(tag, false, false, "dynamic_provider.tag")?;
            }
            ReleaseChannel::Latest if provider.tag.is_some() => {
                return Err(field_error(
                    ErrorCode::InvalidManifest,
                    "dynamic_provider.tag",
                    "Latest and an explicit release tag are mutually exclusive.",
                ));
            }
            ReleaseChannel::Latest => {}
        }
    }
    match (&layer.patch, layer.kind) {
        (Some(patch), LayerKind::Patch) => {
            validate_id(&patch.base_layer, "patch.base_layer")?;
            validate_relative_path(&patch.input_path, false, false, "patch.input_path")?;
            validate_relative_path(&patch.output_path, false, false, "patch.output_path")?;
            validate_digest(&patch.input_digest, "patch.input_digest")?;
            validate_digest(&patch.output_digest, "patch.output_digest")?;
        }
        (None, LayerKind::Patch) | (Some(_), _) => {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Patch layers require a patch specification; other layer kinds cannot contain one.",
            ));
        }
        (None, _) => {}
    }
    Ok(())
}

fn validate_resolved_manifest(manifest: &GameManifest) -> Result<Vec<String>, FusionError> {
    if manifest
        .layers
        .iter()
        .filter(|layer| matches!(layer.source, LayerSource::LocalLocator { .. }))
        .count()
        > 1
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "v1 supports one local-locator base per installation; volume facts cannot describe multiple bases.",
        ));
    }
    if manifest.layers.is_empty() {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "A resolved manifest needs at least one layer.",
        ));
    }
    let ids: Vec<String> = manifest
        .layers
        .iter()
        .map(|layer| layer.id.clone())
        .collect();
    let dependencies = manifest
        .layers
        .iter()
        .map(|layer| (layer.id.clone(), layer.depends_on.clone()))
        .collect();
    let order = topological_order(&ids, &dependencies, ErrorCode::LayerCycle)?;
    validate_patch_integrity(&manifest.layers, &order, false)?;
    let execution_groups = usize::from(manifest.runtime.is_some())
        + usize::from(manifest.launch.is_some())
        + usize::from(manifest.orchestrator.is_some());
    if execution_groups != 1 {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Declare exactly one runtime, launch or orchestrator execution group.",
        ));
    }
    if let Some(runtime) = &manifest.runtime {
        let layer = manifest
            .layers
            .iter()
            .find(|layer| layer.id == runtime.layer_id)
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::MissingLayer,
                    "Runtime references an unknown layer.",
                )
            })?;
        if layer.kind != LayerKind::Runtime {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "runtime.layer_id must reference a runtime layer.",
            ));
        }
        if runtime.default_channel == RuntimeChannel::Dynamic && layer.dynamic_provider.is_none() {
            return Err(FusionError::new(
                ErrorCode::InvalidRuntimeChannel,
                "Dynamic channel requires a provider.",
            ));
        }
        validate_executable(manifest, &runtime.executable)?;
    }
    if let Some(process) = &manifest.launch {
        if !process.depends_on.is_empty() || process.role != ProcessRole::Primary {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "A single launch process must be primary with no dependencies.",
            ));
        }
        validate_executable(manifest, &process.executable)?;
    }
    if let Some(orchestrator) = &manifest.orchestrator {
        if orchestrator.processes.is_empty()
            || orchestrator.processes.len() > 128
            || orchestrator
                .processes
                .iter()
                .filter(|process| process.role == ProcessRole::Primary)
                .count()
                != 1
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "An orchestrator needs one primary process and at most 128 processes.",
            ));
        }
        let mut process_ids = BTreeSet::new();
        for process in &orchestrator.processes {
            unique(&mut process_ids, &process.id, "orchestrator.processes.id")?;
            validate_executable(manifest, &process.executable)?;
        }
        let ids = orchestrator
            .processes
            .iter()
            .map(|process| process.id.clone())
            .collect::<Vec<_>>();
        let deps = orchestrator
            .processes
            .iter()
            .map(|process| (process.id.clone(), process.depends_on.clone()))
            .collect();
        topological_order(&ids, &deps, ErrorCode::InvalidManifest)?;
    }
    for template in &manifest.config_templates {
        validate_template_variables(&template.template, manifest)?;
    }
    for step in &manifest.post_setup_steps {
        let PostSetupStep::WasmTransform {
            layer_id,
            optional,
            fallback_template,
            ..
        } = step;
        if !ids.contains(layer_id) {
            return Err(FusionError::new(
                ErrorCode::MissingLayer,
                "Wasm step references an unknown layer.",
            )
            .with_detail("layer_id", layer_id));
        }
        let capability = "ext.wasm_transform.v1";
        if !manifest
            .capabilities
            .required
            .iter()
            .chain(&manifest.capabilities.optional)
            .any(|item| item == capability)
            || (!optional
                && !manifest
                    .capabilities
                    .required
                    .iter()
                    .any(|item| item == capability))
        {
            return Err(FusionError::new(
                ErrorCode::InvalidCapability,
                "Wasm steps must declare the corresponding required or optional capability.",
            ));
        }
        if let Some(id) = fallback_template {
            if !manifest
                .config_templates
                .iter()
                .any(|template| &template.id == id)
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Wasm fallback references an unknown template.",
                ));
            }
        }
    }
    Ok(order)
}

pub(crate) fn validate_selected_patch_integrity(
    manifest: &CanonicalGameManifest,
    selected: &[String],
) -> Result<(), FusionError> {
    let layers: Vec<LayerSpec> = selected
        .iter()
        .map(|id| manifest.layers[id].clone().into())
        .collect();
    validate_patch_integrity(&layers, selected, true)
}

fn validate_patch_integrity(
    layers: &[LayerSpec],
    order: &[String],
    include_optional: bool,
) -> Result<(), FusionError> {
    let mut critical: BTreeMap<String, String> = BTreeMap::new();
    for id in order {
        let layer = layers
            .iter()
            .find(|layer| &layer.id == id)
            .expect("validated layer graph");
        if layer.optional && !include_optional {
            continue;
        }
        if let Some(patch) = &layer.patch {
            let input = patch.input_path.to_lowercase();
            let output = patch.output_path.to_lowercase();
            let updated = layer
                .critical_files
                .iter()
                .find(|file| mounted_path(&layer.mount_path, &file.path).to_lowercase() == output);
            if (critical.contains_key(&input) || critical.contains_key(&output))
                && updated.is_none()
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "A patch affecting a critical file must declare its updated critical BLAKE3.",
                )
                .with_detail("layerId", id)
                .with_detail("path", &patch.output_path));
            }
            if let Some(expected) = critical.get(&input) {
                if patch.input_digest.algorithm == DigestAlgorithm::Blake3
                    && !patch.input_digest.value.eq_ignore_ascii_case(expected)
                {
                    return Err(FusionError::new(
                        ErrorCode::InvalidDigest,
                        "Patch input digest contradicts the current critical-file digest.",
                    )
                    .with_detail("layerId", id));
                }
            }
            if let Some(file) = updated {
                if patch.output_digest.algorithm == DigestAlgorithm::Blake3
                    && !patch.output_digest.value.eq_ignore_ascii_case(&file.blake3)
                {
                    return Err(FusionError::new(
                        ErrorCode::InvalidDigest,
                        "Patch output digest contradicts its critical-file digest.",
                    )
                    .with_detail("layerId", id));
                }
            }
        }
        let locator_files = match &layer.source {
            LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
            _ => &[],
        };
        for file in locator_files.iter().chain(&layer.critical_files) {
            critical.insert(
                mounted_path(&layer.mount_path, &file.path).to_lowercase(),
                file.blake3.clone(),
            );
        }
    }
    Ok(())
}

fn validate_executable(manifest: &GameManifest, executable: &str) -> Result<(), FusionError> {
    let found = manifest
        .layers
        .iter()
        .filter(|layer| !layer.optional)
        .any(|layer| {
            let files = layer.critical_files.iter().chain(match &layer.source {
                LayerSource::LocalLocator { locator } => locator.key_files.iter(),
                _ => [].iter(),
            });
            files.into_iter().any(|file| {
                file.role == CriticalFileRole::MainExecutable
                    && mounted_path(&layer.mount_path, &file.path) == executable
            })
        });
    if !found {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Each process executable needs a main_executable critical file in a required layer.",
        )
        .with_detail("executable", executable));
    }
    Ok(())
}

pub(crate) fn mounted_path(mount: &str, relative: &str) -> String {
    if mount.is_empty() {
        relative.to_owned()
    } else {
        format!("{mount}/{relative}")
    }
}

pub(crate) fn topological_order(
    ids: &[String],
    dependencies: &BTreeMap<String, Vec<String>>,
    cycle_code: ErrorCode,
) -> Result<Vec<String>, FusionError> {
    let known: BTreeSet<&String> = ids.iter().collect();
    for (id, deps) in dependencies {
        for dependency in deps {
            if !known.contains(dependency) {
                return Err(
                    FusionError::new(ErrorCode::MissingLayer, "Unknown graph dependency.")
                        .with_detail("node", id)
                        .with_detail("dependency", dependency),
                );
            }
        }
    }
    let mut emitted = BTreeSet::new();
    let mut order = Vec::with_capacity(ids.len());
    while order.len() < ids.len() {
        let next = ids.iter().find(|id| {
            !emitted.contains(*id)
                && dependencies
                    .get(*id)
                    .is_none_or(|deps| deps.iter().all(|dep| emitted.contains(dep)))
        });
        let Some(next) = next else {
            return Err(
                FusionError::new(cycle_code, "Cyclic graph dependency.").with_detail(
                    "nodes",
                    ids.iter()
                        .filter(|id| !emitted.contains(*id))
                        .collect::<Vec<_>>(),
                ),
            );
        };
        emitted.insert(next.clone());
        order.push(next.clone());
    }
    Ok(order)
}

fn validate_capabilities(
    requirements: &CapabilityRequirements,
    fallbacks: &[CapabilityFallback],
) -> Result<(), FusionError> {
    let mut names = BTreeSet::new();
    for name in requirements.required.iter().chain(&requirements.optional) {
        validate_capability_name(name)?;
        unique(&mut names, name, "capabilities")?;
    }
    let mut targets = BTreeSet::new();
    for fallback in fallbacks {
        unique(&mut targets, &fallback.capability, "fallback.capability")?;
        if !names.contains(&fallback.capability) {
            return Err(FusionError::new(
                ErrorCode::InvalidCapability,
                "A fallback must target a declared capability.",
            ));
        }
        if fallback.strategy == FallbackStrategy::SkipOptional
            && requirements.required.contains(&fallback.capability)
        {
            return Err(FusionError::new(
                ErrorCode::InvalidCapability,
                "A required capability cannot fall back to skip_optional.",
            ));
        }
        match fallback.strategy {
            FallbackStrategy::HardlinkProjection if !fallback.capability.starts_with("vfs.") => {
                return Err(FusionError::new(
                    ErrorCode::InvalidCapability,
                    "Hardlink projection is a VFS fallback.",
                ));
            }
            FallbackStrategy::BuiltinConfigTemplates
                if fallback.capability != "ext.wasm_transform.v1" =>
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidCapability,
                    "Builtin config templates replace Wasm transforms.",
                ));
            }
            _ => {}
        }
        let mut dependencies = BTreeSet::new();
        for name in &fallback.requires {
            validate_capability_name(name)?;
            unique(&mut dependencies, name, "fallback.requires")?;
        }
    }
    Ok(())
}

fn validate_capability_name(name: &str) -> Result<(), FusionError> {
    let parts: Vec<_> = name.split('.').collect();
    if parts.len() < 2
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
        || !parts.last().is_some_and(|part| {
            part.strip_prefix('v').is_some_and(|version| {
                !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit())
            })
        })
    {
        return Err(field_error(
            ErrorCode::InvalidCapability,
            "capabilities",
            "Expected a versioned capability such as core.layers.v1.",
        ));
    }
    Ok(())
}

fn validate_process(process: &ProcessSpec) -> Result<(), FusionError> {
    validate_id(&process.id, "process.id")?;
    validate_relative_path(&process.executable, false, false, "process.executable")?;
    validate_arguments(&process.args)?;
    validate_environment(&process.env)?;
    let mut deps = BTreeSet::new();
    for id in &process.depends_on {
        validate_id(id, "process.depends_on")?;
        unique(&mut deps, id, "process.depends_on")?;
    }
    Ok(())
}

fn validate_arguments(args: &[String]) -> Result<(), FusionError> {
    if args.iter().any(|arg| arg.chars().any(char::is_control)) {
        return Err(field_error(
            ErrorCode::InvalidManifest,
            "args",
            "Arguments cannot contain control characters.",
        ));
    }
    Ok(())
}

fn validate_environment(env: &BTreeMap<String, String>) -> Result<(), FusionError> {
    let mut names = BTreeSet::new();
    for (name, value) in env {
        if name.is_empty()
            || name.starts_with(|c: char| c.is_ascii_digit())
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.contains('\0')
            || !names.insert(name.to_ascii_uppercase())
        {
            return Err(field_error(
                ErrorCode::InvalidManifest,
                "env",
                "Invalid or duplicate Windows environment variable.",
            ));
        }
    }
    Ok(())
}

fn validate_template_variables(template: &str, manifest: &GameManifest) -> Result<(), FusionError> {
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        if rest[..start].contains("}}") {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Unmatched template delimiter.",
            ));
        }
        rest = &rest[start + 2..];
        let end = rest.find("}}").ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidManifest, "Unclosed template variable.")
        })?;
        let variable = &rest[..end];
        let supported = matches!(
            variable,
            "WORKSPACE_DIR"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "DISPLAY_WIDTH"
                | "DISPLAY_HEIGHT"
                | "REFRESH_RATE"
        ) || variable.strip_prefix("ADDON_ENABLED:").is_some_and(|id| {
            manifest
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.optional)
        });
        if !supported {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Unknown config template variable.",
            )
            .with_detail("variable", variable));
        }
        rest = &rest[end + 2..];
    }
    if rest.contains("}}") {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Unmatched template delimiter.",
        ));
    }
    Ok(())
}

fn validate_critical_files(files: &[CriticalFileSpec], field: &str) -> Result<(), FusionError> {
    let mut paths = BTreeSet::new();
    for file in files {
        validate_relative_path(&file.path, false, false, field)?;
        unique(&mut paths, &file.path.to_ascii_lowercase(), field)?;
        validate_hash(&file.blake3, field)?;
    }
    Ok(())
}

fn validate_digest(digest: &DigestSpec, field: &str) -> Result<(), FusionError> {
    validate_hash(&digest.value, field)
}

fn validate_hash(value: &str, field: &str) -> Result<(), FusionError> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(field_error(
            ErrorCode::InvalidDigest,
            field,
            "BLAKE3 and SHA-256 digests must contain exactly 64 hexadecimal characters.",
        ));
    }
    Ok(())
}

fn validate_mirror(value: &str) -> Result<(), FusionError> {
    let uri = Url::parse(value)
        .map_err(|e| field_error(ErrorCode::InvalidUri, "mirrors", &e.to_string()))?;
    match uri.scheme() {
        "https" => validate_https(value, "mirrors"),
        "magnet" => {
            if !value.starts_with("magnet:?")
                || !uri.query_pairs().any(|(key, value)| {
                    key == "xt"
                        && value.strip_prefix("urn:btih:").is_some_and(|hash| {
                            (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                                || (hash.len() == 32
                                    && hash.bytes().all(|b| {
                                        b.is_ascii_alphabetic() && b.to_ascii_uppercase() <= b'Z'
                                            || (b'2'..=b'7').contains(&b)
                                    }))
                        })
                })
            {
                return Err(field_error(
                    ErrorCode::InvalidUri,
                    "mirrors",
                    "Magnet requires a valid BitTorrent v1 info hash.",
                ));
            }
            for (key, value) in uri.query_pairs() {
                if matches!(key.as_ref(), "ws" | "xs") {
                    validate_https(&value, "magnet.webseed")?;
                }
            }
            Ok(())
        }
        _ => super::adapters::validate_cloud_uri(value).map(drop),
    }
}

fn validate_https(value: &str, field: &str) -> Result<(), FusionError> {
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(field_error(
            ErrorCode::InvalidUri,
            field,
            "URL contains whitespace or control characters.",
        ));
    }
    let uri =
        Url::parse(value).map_err(|e| field_error(ErrorCode::InvalidUri, field, &e.to_string()))?;
    if uri.scheme() != "https"
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.host_str().is_none()
    {
        return Err(field_error(
            ErrorCode::InvalidUri,
            field,
            "Expected an HTTPS URL without embedded credentials.",
        ));
    }
    let host = uri
        .host_str()
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .trim_end_matches('.');
    let private_ip = host.parse::<IpAddr>().is_ok_and(|ip| match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || ip.to_ipv4_mapped().is_some()
        }
    });
    if private_ip
        || host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
    {
        return Err(field_error(
            ErrorCode::InvalidUri,
            field,
            "Local/private HTTPS targets are not source URLs.",
        ));
    }
    Ok(())
}

fn validate_locator_hint(value: &str) -> Result<(), FusionError> {
    nonempty(value, "local_locator.path_hints")?;
    if value.chars().any(char::is_control) || value.contains(['*', '?']) {
        return Err(field_error(
            ErrorCode::InvalidPath,
            "local_locator.path_hints",
            "Locator hints must be explicit paths.",
        ));
    }
    let normalized = value.replace('\\', "/");
    if normalized.split('/').any(|part| part == "..") {
        return Err(field_error(
            ErrorCode::InvalidPath,
            "local_locator.path_hints",
            "Locator hints cannot traverse parent directories.",
        ));
    }
    Ok(())
}

/// Lexical Windows-safe containment. The executor must additionally verify
/// directory handles/reparse points: lexical validation alone cannot do that.
pub(crate) fn validate_relative_path(
    value: &str,
    glob: bool,
    allow_root: bool,
    field: &str,
) -> Result<(), FusionError> {
    if allow_root && value.is_empty() {
        return Ok(());
    }
    if value.is_empty()
        || value.contains('\\')
        || value.starts_with('/')
        || value.contains(':')
        || value.chars().any(char::is_control)
        || value.contains(['<', '>', '"', '|'])
        || (!glob && value.contains(['*', '?', '[', ']']))
    {
        return Err(field_error(
            ErrorCode::InvalidPath,
            field,
            "Expected a safe, relative, forward-slash path.",
        ));
    }
    for part in value.split('/') {
        if part.is_empty() || matches!(part, "." | "..") || part.ends_with(['.', ' ']) {
            return Err(field_error(
                ErrorCode::InvalidPath,
                field,
                "Path traversal, empty segments and trailing dots/spaces are forbidden.",
            ));
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(field_error(
                ErrorCode::InvalidPath,
                field,
                "Windows device names are forbidden.",
            ));
        }
    }
    Ok(())
}

fn qualified_id(source: &str, reference: &str) -> Result<QualifiedGameId, FusionError> {
    let parts: Vec<_> = reference.split("::").collect();
    let (source_id, game_id) = match parts.as_slice() {
        [game] => (source, *game),
        [source, game] => (*source, *game),
        _ => {
            return Err(FusionError::new(
                ErrorCode::InvalidId,
                "Expected game_id or source_id::game_id.",
            ))
        }
    };
    validate_id(source_id, "extends.source_id")?;
    validate_id(game_id, "extends.game_id")?;
    Ok(QualifiedGameId {
        source_id: source_id.to_owned(),
        game_id: game_id.to_owned(),
    })
}

fn validate_id(value: &str, field: &str) -> Result<(), FusionError> {
    if value.is_empty()
        || value.len() > 128
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(field_error(
            ErrorCode::InvalidId,
            field,
            "IDs must be 1-128 ASCII letters, digits, dots, hyphens or underscores.",
        ));
    }
    Ok(())
}

fn nonempty(value: &str, field: &str) -> Result<(), FusionError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(field_error(
            ErrorCode::InvalidManifest,
            field,
            "Expected nonempty text without control characters.",
        ));
    }
    Ok(())
}

fn unique(ids: &mut BTreeSet<String>, value: &str, field: &str) -> Result<(), FusionError> {
    if !ids.insert(value.to_owned()) {
        return Err(
            field_error(ErrorCode::DuplicateId, field, "Duplicate identifier.")
                .with_detail("id", value),
        );
    }
    Ok(())
}

fn field_error(code: ErrorCode, field: &str, message: &str) -> FusionError {
    FusionError::new(code, message).with_detail("field", field)
}
