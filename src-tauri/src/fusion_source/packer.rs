//! Offline source authoring. Recipes describe local inputs; published manifests
//! use the same v1 wire contracts and validators as the launcher. No recipe can
//! execute a shell command, download an executable, or grant publisher trust.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::fusion_creator::projection::{
    resolve_projection, ProjectionLayer, ProjectionRequest, ProjectionSemantics,
};

use super::canonical::{CanonicalGameManifest, CanonicalLayer, QualifiedGameId};
use super::capabilities::{CapabilityNegotiator, CapabilityRegistry};
use super::cas::{hash_file, CasFile, CasObject};
use super::config_templates::{ConfigTemplateEngine, TemplateContext};
use super::errors::{ErrorCode, FusionError};
use super::evolution::SchemaUpcaster;
use super::fs_safety::{self, io_error};
use super::layers::LayerCache;
use super::plan::{PlanBuilder, PlanOptions};
use super::resolver::DynamicAssetAttestation;
use super::schema::*;
use super::signatures;

#[cfg(test)]
#[path = "packer_projection_tests.rs"]
mod projection_tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerRecipe {
    #[serde(alias = "source")]
    pub source_info: SourceInfo,
    /// Optional HTTPS publication root, used for manifest and default artifact URLs.
    #[serde(default)]
    pub base_url: Option<String>,
    pub games: Vec<PackerGameRecipe>,
    /// Explicit local parent manifests, including cross-source parents.
    #[serde(default)]
    pub parents: Vec<PackerParentRecipe>,
    /// Explicit capabilities of the intended client, not host adapter discovery.
    #[serde(default)]
    pub target_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerParentRecipe {
    pub source_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerGameRecipe {
    pub id: String,
    pub title: String,
    pub version: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub capabilities: CapabilityRequirements,
    #[serde(default)]
    pub fallback: Vec<CapabilityFallback>,
    #[serde(default)]
    pub layers: Vec<PackerLayerRecipe>,
    #[serde(default)]
    pub runtime: Option<RuntimeSpec>,
    #[serde(default)]
    pub launch: Option<ProcessSpec>,
    #[serde(default)]
    pub orchestrator: Option<OrchestratorSpec>,
    #[serde(default)]
    pub persistence: PersistenceSpec,
    #[serde(default)]
    pub config_templates: Vec<ConfigTemplateSpec>,
    #[serde(default)]
    pub post_setup_steps: Vec<PostSetupStep>,
    #[serde(default)]
    pub local_placement: LocalPlacementPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerLayerRecipe {
    pub id: String,
    #[serde(default, alias = "name", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: LayerKind,
    #[serde(default)]
    pub content: LayerContent,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Relative to the recipe, or an explicit absolute plain file/directory.
    #[serde(default, alias = "path")]
    pub local_path: Option<PathBuf>,
    /// Directories default to deterministic ZIP. Files default to their extension.
    #[serde(default)]
    pub archive: Option<ArchiveFormat>,
    #[serde(default)]
    pub raw_path: Option<String>,
    #[serde(default)]
    pub mirrors: Vec<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub critical_files: Vec<PackerCriticalFile>,
    /// Select the sole .exe in the inspected tree. Ambiguity is an error.
    #[serde(default)]
    pub auto_main_executable: bool,
    #[serde(default)]
    pub mount_path: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub default_enabled: bool,
    #[serde(default)]
    pub writable_copy_paths: Vec<String>,
    #[serde(default)]
    pub local_locator: Option<PackerLocatorRecipe>,
    #[serde(default)]
    pub dynamic_provider: Option<GithubReleaseProvider>,
    #[serde(default)]
    pub attestation: Option<PackerAttestationRecipe>,
    #[serde(default)]
    pub patch: Option<PackerPatchRecipe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerPatchRecipe {
    pub format: PatchFormat,
    pub base_layer: String,
    /// Workspace-relative paths, matching the core patch execution contract.
    pub input_path: String,
    pub output_path: String,
    pub input_reference: PathBuf,
    pub output_reference: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerCriticalFile {
    pub path: String,
    pub role: CriticalFileRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerLocatorRecipe {
    pub reference_dir: PathBuf,
    pub store: LocalStore,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub path_hints: Vec<String>,
    pub key_files: Vec<PackerCriticalFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackerAttestationRecipe {
    pub release_id: String,
    #[serde(default, alias = "tag")]
    pub release_tag: Option<String>,
    pub asset_id: String,
    pub asset_url: String,
    #[serde(default)]
    pub tool_versions: BTreeMap<String, String>,
}

/// Exported source-policy input, not a signature or an automatic trust grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAttestedRelease {
    pub release_id: String,
    pub tag: String,
    pub asset_id: String,
    pub asset_url: String,
    pub transport_digest: DigestSpec,
    pub critical_files: Vec<CriticalFileSpec>,
    pub tool_versions: BTreeMap<String, String>,
}

impl From<SourceAttestedRelease> for DynamicAssetAttestation {
    fn from(value: SourceAttestedRelease) -> Self {
        Self {
            release_id: value.release_id,
            tag: value.tag,
            asset_id: value.asset_id,
            asset_url: value.asset_url,
            transport_digest: value.transport_digest,
            critical_files: value.critical_files,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArtifactInspectionOptions {
    pub archive: Option<ArchiveFormat>,
    pub raw_path: Option<String>,
    pub critical_files: Vec<PackerCriticalFile>,
    pub auto_main_executable: bool,
    pub attestation: Option<PackerAttestationRecipe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInspection {
    pub archive: ArchiveFormat,
    pub raw_path: Option<String>,
    pub transport_digest: DigestSpec,
    pub size_bytes: u64,
    pub critical_files: Vec<CriticalFileSpec>,
    pub cas_digest: String,
    pub attestation: Option<SourceAttestedRelease>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltArtifact {
    pub game_id: String,
    pub layer_id: String,
    pub path: PathBuf,
    pub raw_path: Option<String>,
    pub transport_digest: DigestSpec,
    pub size_bytes: u64,
    pub cas_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildReport {
    pub source_index: PathBuf,
    pub manifests: BTreeMap<String, PathBuf>,
    pub artifacts: Vec<BuiltArtifact>,
    pub attestations: Vec<PathBuf>,
    /// The schema's locator key spec has no size field; inventory keeps sizes here.
    pub locator_files: BTreeMap<String, Vec<CasFile>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyReport {
    pub source_id: String,
    pub signature_verified: bool,
    pub manifests_verified: usize,
    pub artifacts_verified: usize,
    pub attestations_verified: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactBinding {
    layer_id: String,
    path: String,
    raw_path: Option<String>,
    cas_digest: String,
    attestation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestInventory {
    artifacts: Vec<ArtifactBinding>,
    /// Parent bytes are exported beside the manifest so offline verification
    /// can resolve even explicitly supplied cross-source ancestry.
    parents: Vec<PublishedParent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    target_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishedParent {
    source_id: String,
    path: String,
    blake3: String,
}

#[derive(Debug, Default)]
pub struct PackerEngine;

impl PackerEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn read_recipe(path: &Path) -> Result<PackerRecipe, FusionError> {
        let path = absolute(path)?;
        let text = read_text(&path)?;
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
        {
            toml::from_str(&text)
                .map_err(|error| FusionError::new(ErrorCode::InvalidManifest, error.to_string()))
        } else {
            serde_json::from_str(&text)
                .map_err(|error| FusionError::new(ErrorCode::InvalidJson, error.to_string()))
        }
    }

    pub fn build_file(&self, recipe_path: &Path, out: &Path) -> Result<BuildReport, FusionError> {
        self.build_file_with_signing_key(recipe_path, out, None)
    }

    pub fn build_file_with_signing_key(
        &self,
        recipe_path: &Path,
        out: &Path,
        signing_key: Option<&Path>,
    ) -> Result<BuildReport, FusionError> {
        let path = absolute(recipe_path)?;
        let recipe = Self::read_recipe(&path)?;
        self.build_with_signing_key(
            &recipe,
            path.parent()
                .ok_or_else(|| invalid_path("Recipe has no parent."))?,
            out,
            signing_key,
        )
    }

    pub fn inspect_artifact(
        &self,
        file: &Path,
        options: &ArtifactInspectionOptions,
    ) -> Result<ArtifactInspection, FusionError> {
        let scratch = Scratch::new()?;
        let cache = LayerCache::open(&scratch.0)?;
        let (inspection, _) =
            inspect_with_cache(&cache, &absolute(file)?, options, LayerKind::Base)?;
        Ok(inspection)
    }

    pub fn build(
        &self,
        recipe: &PackerRecipe,
        recipe_dir: &Path,
        out: &Path,
    ) -> Result<BuildReport, FusionError> {
        self.build_with_signing_key(recipe, recipe_dir, out, None)
    }

    pub fn build_with_signing_key(
        &self,
        recipe: &PackerRecipe,
        recipe_dir: &Path,
        out: &Path,
        signing_key: Option<&Path>,
    ) -> Result<BuildReport, FusionError> {
        use base64::Engine;
        let signing_key = signing_key.map(signatures::read_signing_key).transpose()?;
        let mut effective_recipe = recipe.clone();
        if let Some(key) = &signing_key {
            let public =
                base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
            if recipe
                .source_info
                .public_key
                .as_ref()
                .is_some_and(|declared| declared != &public)
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidSourceSignature,
                    "The recipe public key differs from the signing key.",
                ));
            }
            effective_recipe.source_info.public_key = Some(public);
        } else if recipe.source_info.public_key.is_some() {
            return Err(FusionError::new(
                ErrorCode::InvalidSourceSignature,
                "A recipe declaring a public key requires a signing key.",
            ));
        }
        let recipe = &effective_recipe;
        validate_recipe_names(recipe)?;
        let recipe_dir = absolute(recipe_dir)?;
        let _recipe_root = fs_safety::guard(&recipe_dir)?;
        let out = absolute(out)?;
        let _out = fs_safety::ensure_directory(&out)?;
        let _lock = RepositoryLock::acquire(&out)?;
        let scratch = Scratch::new_in(&out)?;
        let cache = LayerCache::open(&scratch.0.join("inspection"))?;
        let index_path = out.join("source-index.json");
        let mut index = if index_path
            .try_exists()
            .map_err(|e| io_error("Inspect index", &index_path, e))?
        {
            let text = read_text(&index_path)?;
            let original = signatures::parse_wire_index(&text)?;
            signatures::verify_wire_index(&original, None, None)?;
            SchemaUpcaster::upcast_source_index(&text)?;
            let previous: SourceIndex = parse_json(&text)?;
            if previous.source_info.id != recipe.source_info.id {
                return Err(FusionError::new(
                    ErrorCode::InvalidId,
                    "Cannot update a repository belonging to another source.",
                ));
            }
            if previous.source_info.public_key != recipe.source_info.public_key {
                return Err(FusionError::new(
                    ErrorCode::InvalidSourceSignature,
                    "Updating a repository cannot replace or remove its public key.",
                ));
            }
            previous
        } else {
            SourceIndex {
                schema_version: SCHEMA_VERSION.into(),
                source_info: recipe.source_info.clone(),
                games: Vec::new(),
                signature: None,
            }
        };
        let existing_index = index.clone();
        let mut parents = load_index_manifests(&out, &index)?;
        for parent in &recipe.parents {
            let path = resolve_input(&recipe_dir, &parent.path)?;
            let manifest = SchemaUpcaster::parse_game_manifest(&read_text(&path)?)?;
            let key = QualifiedGameId {
                source_id: parent.source_id.clone(),
                game_id: manifest.id.clone(),
            };
            if parents
                .get(&key)
                .is_some_and(|existing| existing != &manifest)
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Conflicting definitions of an authoring parent.",
                ));
            }
            parents.insert(key, manifest);
        }
        let mut report = BuildReport {
            source_index: index_path.clone(),
            manifests: BTreeMap::new(),
            artifacts: Vec::new(),
            attestations: Vec::new(),
            locator_files: BTreeMap::new(),
        };
        let mut pending = Vec::new();
        let mut inventories = BTreeMap::new();
        let mut recipe_bindings = BTreeMap::new();
        for game in &recipe.games {
            let mut layers = Vec::new();
            let mut bindings = Vec::new();
            for layer in &game.layers {
                let mut spec = layer_spec(layer);
                if let Some(patch) = &layer.patch {
                    spec.patch = Some(author_patch(patch, &recipe_dir)?);
                }
                if let Some(locator) = &layer.local_locator {
                    if layer.local_path.is_some()
                        || layer.archive.is_some()
                        || !layer.mirrors.is_empty()
                        || layer.base_url.is_some()
                        || layer.dynamic_provider.is_some()
                        || layer.attestation.is_some()
                    {
                        return Err(FusionError::new(
                            ErrorCode::InvalidManifest,
                            "A locator layer cannot also publish an artifact or provider.",
                        ));
                    }
                    let root = resolve_input(&recipe_dir, &locator.reference_dir)?;
                    reject_output_inside_source(&root, &out)?;
                    let files = inventory_local_directory(&root)?;
                    let keys = critical_specs(&files, &locator.key_files, false)?;
                    spec.critical_files =
                        critical_specs(&files, &layer.critical_files, layer.auto_main_executable)?;
                    spec.source = LayerSource::LocalLocator {
                        locator: LocalLocatorSpec {
                            store: locator.store,
                            app_id: locator.app_id.clone(),
                            path_hints: locator.path_hints.clone(),
                            key_files: keys,
                        },
                    };
                    report
                        .locator_files
                        .insert(format!("{}::{}", game.id, layer.id), files.clone());
                    inventories.insert((game.id.clone(), layer.id.clone()), files);
                } else {
                    let local = layer.local_path.as_ref().ok_or_else(|| {
                        FusionError::new(
                            ErrorCode::InvalidManifest,
                            "An artifact layer requires local_path.",
                        )
                    })?;
                    let input = resolve_input(&recipe_dir, local)?;
                    let metadata = fs::symlink_metadata(&input)
                        .map_err(|e| io_error("Inspect layer input", &input, e))?;
                    fs_safety::check_metadata(&metadata, &input)?;
                    let mut raw_path = layer.raw_path.clone();
                    let format = layer.archive.unwrap_or_else(|| {
                        if metadata.is_dir() {
                            ArchiveFormat::Zip
                        } else {
                            infer_archive(&input)
                        }
                    });
                    let staged = scratch.0.join(fs_safety::unique_name("artifact"));
                    if metadata.is_dir() {
                        reject_output_inside_source(&input, &out)?;
                        let object = cache.object(&cache.prepare_directory(&input)?)?;
                        match format {
                            ArchiveFormat::Zip => write_deterministic_zip(&object, &staged)?,
                            ArchiveFormat::Raw => {
                                if object.files.len() != 1 { return Err(FusionError::new(ErrorCode::InvalidManifest, "A raw directory layer must contain exactly one file.")); }
                                let file = &object.files[0];
                                raw_path.get_or_insert_with(|| file.path.clone());
                                fs_safety::copy_file(&object.root.join(fs_safety::validate_relative_path(&file.path)?), &staged)?;
                            }
                            ArchiveFormat::SevenZip => return Err(FusionError::new(ErrorCode::ArchiveInvalid, "Directory authoring supports zip or raw; supply a prebuilt 7z file.")),
                        }
                    } else {
                        if format == ArchiveFormat::Raw && raw_path.is_none() {
                            raw_path = Some(file_name(&input)?);
                        }
                        fs_safety::copy_file(&input, &staged)?;
                    }
                    let options = ArtifactInspectionOptions {
                        archive: Some(format),
                        raw_path: raw_path.clone(),
                        critical_files: if spec.patch.is_some() {
                            Vec::new()
                        } else {
                            layer.critical_files.clone()
                        },
                        auto_main_executable: spec.patch.is_none() && layer.auto_main_executable,
                        attestation: layer.attestation.clone(),
                    };
                    let (inspection, object) =
                        inspect_with_cache(&cache, &staged, &options, layer.kind)?;
                    let filename = format!(
                        "{}.{}",
                        inspection.transport_digest.value,
                        archive_extension(format)
                    );
                    let relative = format!("artifacts/{filename}");
                    let mirrors = artifact_mirrors(layer, recipe.base_url.as_deref(), &filename)?;
                    spec.source = LayerSource::Remote {
                        mirrors,
                        archive: format,
                    };
                    spec.transport_digest = Some(inspection.transport_digest.clone());
                    spec.size_bytes = Some(inspection.size_bytes);
                    spec.critical_files = inspection.critical_files.clone();
                    if let Some(patch) = &spec.patch {
                        if object.files.len() != 1 {
                            return Err(FusionError::new(
                                ErrorCode::InvalidManifest,
                                "A patch layer must contain exactly one patch payload.",
                            ));
                        }
                        spec.critical_files = patch_critical_specs(layer, patch)?;
                    }
                    if layer.dynamic_provider.is_some() && inspection.attestation.is_none() {
                        return Err(FusionError::new(
                            ErrorCode::ProviderResolutionFailed,
                            "A dynamic provider requires a locally generated release attestation.",
                        ));
                    }
                    if layer.dynamic_provider.is_none() && inspection.attestation.is_some() {
                        return Err(FusionError::new(
                            ErrorCode::InvalidManifest,
                            "A release attestation requires a dynamic provider.",
                        ));
                    }
                    if let (Some(provider), Some(attestation)) =
                        (&layer.dynamic_provider, &inspection.attestation)
                    {
                        validate_provider_attestation(provider, attestation)?;
                    }
                    let attestation_relative = inspection
                        .attestation
                        .as_ref()
                        .map(|attestation| {
                            let bytes = canonical_bytes(attestation)?;
                            let digest = blake3::hash(&bytes).to_hex().to_string();
                            let relative = format!("attestations/{digest}.json");
                            pending.push(PendingFile::Bytes(relative.clone(), bytes));
                            report.attestations.push(out.join(&relative));
                            Ok::<_, FusionError>(relative)
                        })
                        .transpose()?;
                    bindings.push(ArtifactBinding {
                        layer_id: layer.id.clone(),
                        path: relative.clone(),
                        raw_path: inspection.raw_path.clone(),
                        cas_digest: inspection.cas_digest.clone(),
                        attestation: attestation_relative,
                    });
                    report.artifacts.push(BuiltArtifact {
                        game_id: game.id.clone(),
                        layer_id: layer.id.clone(),
                        path: out.join(&relative),
                        raw_path: inspection.raw_path,
                        transport_digest: inspection.transport_digest,
                        size_bytes: inspection.size_bytes,
                        cas_digest: inspection.cas_digest,
                    });
                    pending.push(PendingFile::Copy(relative, staged));
                    inventories.insert((game.id.clone(), layer.id.clone()), object.files);
                }
                layers.push(spec);
            }
            let manifest = game_manifest(game, layers);
            let bytes = canonical_bytes(&manifest)?;
            SchemaUpcaster::parse_game_manifest(
                std::str::from_utf8(&bytes)
                    .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?,
            )?;
            parents.insert(
                QualifiedGameId {
                    source_id: recipe.source_info.id.clone(),
                    game_id: game.id.clone(),
                },
                manifest.clone(),
            );
            recipe_bindings.insert(
                QualifiedGameId {
                    source_id: recipe.source_info.id.clone(),
                    game_id: game.id.clone(),
                },
                bindings.clone(),
            );
            pending.push(PendingFile::Game {
                manifest: Box::new(manifest),
                bytes,
                bindings,
            });
        }
        // Existing derived games bind the parent bytes exported with their
        // authoring inventory. Updating a parent alone must not silently alter
        // an unchanged child's effective binaries and break offline verification.
        for entry in &existing_index.games {
            if recipe
                .games
                .iter()
                .any(|game| game.id.eq_ignore_ascii_case(&entry.id))
            {
                continue;
            }
            let path = out.join(inventory_relative(&entry.id, &entry.manifest_blake3));
            if !path
                .try_exists()
                .map_err(|e| io_error("Inspect inherited inventory", &path, e))?
            {
                continue;
            }
            let inventory: ManifestInventory = parse_json(&read_text(&path)?)?;
            for parent in inventory.parents {
                let bytes =
                    read_bytes(&out.join(fs_safety::validate_relative_path(&parent.path)?))?;
                verify_raw_manifest(&bytes, &parent.blake3)?;
                let wire = SchemaUpcaster::parse_game_manifest(utf8(&bytes)?)?;
                let key = QualifiedGameId {
                    source_id: parent.source_id,
                    game_id: wire.id.clone(),
                };
                if parents.get(&key).is_some_and(|current| current != &wire) {
                    return Err(FusionError::new(ErrorCode::InvalidManifest,"Rebuild derived games with their updated parent to preserve inherited artifact bindings.").with_detail("gameId",&entry.id));
                }
            }
        }
        // Resolve the complete graph before publishing a single manifest or index.
        let mut publication = Vec::new();
        let mut aliases = Vec::new();
        for item in pending {
            match item {
                PendingFile::Game {
                    manifest,
                    bytes,
                    mut bindings,
                } => {
                    let manifest = *manifest;
                    let text = std::str::from_utf8(&bytes)
                        .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?;
                    let canonical = self_check(
                        &recipe.source_info.id,
                        text,
                        &parents,
                        &recipe.target_capabilities,
                    )?;
                    // Inherited artifact bindings come from the already published
                    // parent inventories (or another recipe game).
                    let mut inherited = inherited_bindings(
                        &canonical,
                        &out,
                        &existing_index,
                        &recipe_bindings,
                        &parents,
                    )?;
                    inherited.retain(|binding| {
                        !bindings.iter().any(|own| own.layer_id == binding.layer_id)
                            && canonical
                                .layers
                                .get(&binding.layer_id)
                                .is_some_and(|layer| {
                                    matches!(layer.source, LayerSource::Remote { .. })
                                })
                    });
                    bindings.extend(inherited);
                    bindings.sort_by(|a, b| a.layer_id.cmp(&b.layer_id));
                    let mut physical: BTreeMap<_, _> = inventories
                        .iter()
                        .filter(|((game_id, _), _)| game_id == &canonical.id)
                        .map(|(key, files)| (key.clone(), files.clone()))
                        .collect();
                    let ancestors = ancestor_keys(&canonical, &parents)?;
                    for binding in &bindings {
                        if physical.contains_key(&(canonical.id.clone(), binding.layer_id.clone()))
                        {
                            continue;
                        }
                        let layer = &canonical.layers[&binding.layer_id];
                        let own = ancestors.iter().find_map(|key| {
                            inventories.get(&(key.game_id.clone(), binding.layer_id.clone()))
                        });
                        if let Some(files) = own {
                            physical.insert(
                                (canonical.id.clone(), binding.layer_id.clone()),
                                files.clone(),
                            );
                        } else {
                            let archive = match layer.source {
                                LayerSource::Remote { archive, .. } => archive,
                                _ => continue,
                            };
                            let options = ArtifactInspectionOptions {
                                archive: Some(archive),
                                raw_path: binding.raw_path.clone(),
                                ..Default::default()
                            };
                            let (_, object) = inspect_with_cache(
                                &cache,
                                &out.join(fs_safety::validate_relative_path(&binding.path)?),
                                &options,
                                layer.kind,
                            )?;
                            physical.insert(
                                (canonical.id.clone(), binding.layer_id.clone()),
                                object.files,
                            );
                        }
                    }
                    for layer in canonical.layers.values() {
                        if matches!(layer.source, LayerSource::Remote { .. })
                            && !bindings.iter().any(|binding| binding.layer_id == layer.id)
                        {
                            return Err(FusionError::new(ErrorCode::MissingLayer,"An inherited remote layer requires a local artifact binding; build its parent in this repository first."));
                        }
                    }
                    validate_layer_inventory(&canonical, &physical)?;
                    let digest = blake3::hash(&bytes).to_hex().to_string();
                    let relative = format!("manifests/{}-{digest}.json", manifest.id);
                    let mut exported_parents = Vec::new();
                    for key in ancestors {
                        let parent = &parents[&key];
                        let parent_bytes = canonical_bytes(parent)?;
                        let parent_digest = blake3::hash(&parent_bytes).to_hex().to_string();
                        let parent_relative = format!("parents/{parent_digest}.json");
                        exported_parents.push(PublishedParent {
                            source_id: key.source_id.clone(),
                            path: parent_relative.clone(),
                            blake3: parent_digest,
                        });
                        publication.push(PendingFile::Bytes(parent_relative, parent_bytes));
                    }
                    let inventory = ManifestInventory {
                        artifacts: bindings,
                        parents: exported_parents,
                        // A target registry is a set. Recipe ordering and repeated
                        // capabilities must not change an immutable inventory.
                        target_capabilities: recipe
                            .target_capabilities
                            .iter()
                            .cloned()
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                    };
                    publication.push(PendingFile::Bytes(
                        inventory_relative(&manifest.id, &digest),
                        canonical_bytes(&inventory)?,
                    ));
                    publication.push(PendingFile::Bytes(relative.clone(), bytes.clone()));
                    aliases.push((format!("manifests/{}.json", manifest.id), bytes));
                    let entry = SourceGameEntry {
                        id: manifest.id.clone(),
                        title: manifest.title,
                        version: manifest.version,
                        platform: manifest.platform,
                        cover_url: manifest.cover_url,
                        manifest_url: publication_url(recipe.base_url.as_deref(), &relative)?,
                        manifest_blake3: digest,
                    };
                    index
                        .games
                        .retain(|old| !old.id.eq_ignore_ascii_case(&entry.id));
                    index.games.push(entry);
                    report.manifests.insert(
                        manifest.id.clone(),
                        out.join(format!("manifests/{}.json", manifest.id)),
                    );
                }
                other => publication.push(other),
            }
        }
        index.source_info = recipe.source_info.clone();
        index.games.sort_by(|a, b| a.id.cmp(&b.id));
        index.signature = None;
        if let Some(key) = &signing_key {
            signatures::sign_index(&mut index, key)?;
        }
        let index_bytes = canonical_bytes(&index)?;
        SchemaUpcaster::upcast_source_index(
            std::str::from_utf8(&index_bytes)
                .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?,
        )?;
        for item in publication {
            match item {
                PendingFile::Bytes(relative, bytes) => {
                    publish_immutable_bytes(&out, &relative, &bytes)?
                }
                PendingFile::Copy(relative, source) => {
                    publish_immutable_copy(&out, &relative, &source)?
                }
                PendingFile::Game { .. } => unreachable!(),
            }
        }
        // Commit immutable bindings before replacing convenience aliases. This
        // also preserves a previous legacy index that still referenced an alias.
        fs_safety::atomic_write(&index_path, &index_bytes)?;
        for (relative, bytes) in aliases {
            fs_safety::atomic_write(&out.join(relative), &bytes)?;
        }
        Ok(report)
    }

    pub fn verify(&self, source_dir: &Path) -> Result<VerifyReport, FusionError> {
        let root = absolute(source_dir)?;
        let _root = fs_safety::guard(&root)?;
        let text = read_text(&root.join("source-index.json"))?;
        let document = signatures::parse_wire_index(&text)?;
        let signature_verified = signatures::verify_wire_index(&document, None, None)?;
        let canonical_index = SchemaUpcaster::upcast_source_index(&text)?;
        let index: SourceIndex = parse_json(&text)?;
        let mut parents = load_index_manifests(&root, &index)?;
        let scratch = Scratch::new_in(&root)?;
        let cache = LayerCache::open(&scratch.0)?;
        let mut report = VerifyReport {
            source_id: canonical_index.source_info.id.clone(),
            signature_verified,
            manifests_verified: 0,
            artifacts_verified: 0,
            attestations_verified: 0,
        };
        for entry in &index.games {
            let inventory_path = root.join(inventory_relative(&entry.id, &entry.manifest_blake3));
            let inventory: ManifestInventory = parse_json(&read_text(&inventory_path)?)?;
            for parent in &inventory.parents {
                let path = root.join(fs_safety::validate_relative_path(&parent.path)?);
                let bytes = read_bytes(&path)?;
                verify_raw_manifest(&bytes, &parent.blake3)?;
                let manifest = SchemaUpcaster::parse_game_manifest(utf8(&bytes)?)?;
                let key = QualifiedGameId {
                    source_id: parent.source_id.clone(),
                    game_id: manifest.id.clone(),
                };
                if parents.get(&key).is_some_and(|old| old != &manifest) {
                    return Err(FusionError::new(
                        ErrorCode::InvalidManifest,
                        "Conflicting published parent manifest.",
                    ));
                }
                parents.insert(key, manifest);
            }
        }
        for entry in &index.games {
            let manifest_path = indexed_manifest_path(&root, entry)?;
            let bytes = read_bytes(&manifest_path)?;
            verify_raw_manifest(&bytes, &entry.manifest_blake3)?;
            let inventory: ManifestInventory = parse_json(&read_text(
                &root.join(inventory_relative(&entry.id, &entry.manifest_blake3)),
            )?)?;
            let manifest = self_check(
                &index.source_info.id,
                utf8(&bytes)?,
                &parents,
                &inventory.target_capabilities,
            )?;
            let mut seen = BTreeSet::new();
            let mut physical = BTreeMap::new();
            for binding in inventory.artifacts {
                if !seen.insert(binding.layer_id.clone()) {
                    return Err(FusionError::new(
                        ErrorCode::DuplicateId,
                        "Duplicate local artifact binding.",
                    ));
                }
                let layer = manifest.layers.get(&binding.layer_id).ok_or_else(|| {
                    FusionError::new(
                        ErrorCode::MissingLayer,
                        "Local artifact binding names an unknown layer.",
                    )
                })?;
                let archive = match layer.source {
                    LayerSource::Remote { archive, .. } => archive,
                    _ => {
                        return Err(FusionError::new(
                            ErrorCode::InvalidManifest,
                            "Artifact inventory may only bind remote layers.",
                        ))
                    }
                };
                let path = root.join(fs_safety::validate_relative_path(&binding.path)?);
                if !binding.path.starts_with("artifacts/") {
                    return Err(invalid_path(
                        "Local artifacts must remain under artifacts/.",
                    ));
                }
                let options = ArtifactInspectionOptions {
                    archive: Some(archive),
                    raw_path: binding.raw_path,
                    critical_files: if layer.patch.is_some() {
                        Vec::new()
                    } else {
                        layer
                            .critical_files
                            .iter()
                            .map(|file| PackerCriticalFile {
                                path: file.path.clone(),
                                role: file.role,
                            })
                            .collect()
                    },
                    ..Default::default()
                };
                let (inspection, object) = inspect_with_cache(&cache, &path, &options, layer.kind)?;
                if layer.transport_digest.as_ref() != Some(&inspection.transport_digest)
                    || layer.size_bytes != Some(inspection.size_bytes)
                {
                    return Err(FusionError::new(
                        ErrorCode::TransportDigestMismatch,
                        "Published artifact digest or size differs from its manifest.",
                    )
                    .with_detail("layerId", &layer.id));
                }
                if (layer.patch.is_none() && inspection.critical_files != layer.critical_files)
                    || inspection.cas_digest != binding.cas_digest
                {
                    return Err(FusionError::new(
                        ErrorCode::CorruptedCriticalBinary,
                        "Published artifact inventory differs from its manifest.",
                    ));
                }
                if let Some(patch) = &layer.patch {
                    if object.files.len() != 1 {
                        return Err(FusionError::new(
                            ErrorCode::InvalidManifest,
                            "A patch layer must contain exactly one patch payload.",
                        ));
                    }
                    validate_patch_critical(layer, patch)?;
                }
                if let Some(relative) = binding.attestation {
                    let bytes =
                        read_bytes(&root.join(fs_safety::validate_relative_path(&relative)?))?;
                    let digest = blake3::hash(&bytes).to_hex().to_string();
                    if relative != format!("attestations/{digest}.json") {
                        return Err(FusionError::new(
                            ErrorCode::IntegrityViolation,
                            "Published attestation bytes differ from their immutable filename.",
                        ));
                    }
                    let attestation: SourceAttestedRelease = parse_json(utf8(&bytes)?)?;
                    if layer.dynamic_provider.is_none()
                        || attestation.transport_digest != inspection.transport_digest
                        || attestation.critical_files != inspection.critical_files
                    {
                        return Err(FusionError::new(
                            ErrorCode::ProviderResolutionFailed,
                            "Release attestation does not bind this runtime artifact.",
                        ));
                    }
                    validate_attestation(&attestation)?;
                    if let Some(provider) = &layer.dynamic_provider {
                        validate_provider_attestation(provider, &attestation)?;
                    }
                    report.attestations_verified += 1;
                } else if layer.dynamic_provider.is_some() {
                    return Err(FusionError::new(
                        ErrorCode::ProviderResolutionFailed,
                        "Dynamic runtime is missing its source attestation.",
                    ));
                }
                physical.insert((manifest.id.clone(), layer.id.clone()), object.files);
                report.artifacts_verified += 1;
            }
            for layer in manifest.layers.values() {
                if matches!(layer.source, LayerSource::Remote { .. }) && !seen.contains(&layer.id) {
                    return Err(FusionError::new(
                        ErrorCode::MissingLayer,
                        "Published remote layer is missing its local artifact.",
                    ));
                }
            }
            validate_layer_inventory(&manifest, &physical)?;
            report.manifests_verified += 1;
        }
        Ok(report)
    }
}

enum PendingFile {
    Bytes(String, Vec<u8>),
    Copy(String, PathBuf),
    Game {
        manifest: Box<GameManifest>,
        bytes: Vec<u8>,
        bindings: Vec<ArtifactBinding>,
    },
}

fn layer_spec(recipe: &PackerLayerRecipe) -> LayerSpec {
    LayerSpec {
        id: recipe.id.clone(),
        title: recipe.title.clone(),
        description: recipe.description.clone(),
        kind: recipe.kind,
        content: recipe.content,
        depends_on: recipe.depends_on.clone(),
        source: LayerSource::UserProvided {
            instructions: None,
            archive: ArchiveFormat::Raw,
        },
        transport_digest: None,
        size_bytes: None,
        mount_path: recipe.mount_path.clone(),
        optional: recipe.optional,
        default_enabled: recipe.default_enabled,
        writable_copy_paths: recipe.writable_copy_paths.clone(),
        critical_files: Vec::new(),
        dynamic_provider: recipe.dynamic_provider.clone(),
        patch: None,
    }
}

fn game_manifest(game: &PackerGameRecipe, layers: Vec<LayerSpec>) -> GameManifest {
    GameManifest {
        schema_version: SCHEMA_VERSION.into(),
        creator: None,
        id: game.id.clone(),
        title: game.title.clone(),
        version: game.version.clone(),
        platform: game.platform.clone(),
        cover_url: game.cover_url.clone(),
        extends: game.extends.clone(),
        capabilities: game.capabilities.clone(),
        fallback: game.fallback.clone(),
        layers,
        runtime: game.runtime.clone(),
        launch: game.launch.clone(),
        orchestrator: game.orchestrator.clone(),
        persistence: game.persistence.clone(),
        config_templates: game.config_templates.clone(),
        post_setup_steps: game.post_setup_steps.clone(),
        local_placement: game.local_placement,
    }
}

fn author_patch(recipe: &PackerPatchRecipe, recipe_dir: &Path) -> Result<PatchSpec, FusionError> {
    fs_safety::validate_relative_path(&recipe.input_path)?;
    fs_safety::validate_relative_path(&recipe.output_path)?;
    let input = resolve_input(recipe_dir, &recipe.input_reference)?;
    let output = resolve_input(recipe_dir, &recipe.output_reference)?;
    let _input = fs_safety::open_read(&input)?;
    let _output = fs_safety::open_read(&output)?;
    Ok(PatchSpec {
        format: recipe.format,
        base_layer: recipe.base_layer.clone(),
        input_path: recipe.input_path.clone(),
        output_path: recipe.output_path.clone(),
        input_digest: DigestSpec {
            algorithm: DigestAlgorithm::Blake3,
            value: hash_file(&input, DigestAlgorithm::Blake3)?,
        },
        output_digest: DigestSpec {
            algorithm: DigestAlgorithm::Blake3,
            value: hash_file(&output, DigestAlgorithm::Blake3)?,
        },
    })
}

fn mounted_path(mount: &str, path: &str) -> String {
    if mount.is_empty() {
        path.into()
    } else {
        format!("{mount}/{path}")
    }
}

fn patch_critical_specs(
    recipe: &PackerLayerRecipe,
    patch: &PatchSpec,
) -> Result<Vec<CriticalFileSpec>, FusionError> {
    for file in &recipe.critical_files {
        fs_safety::validate_relative_path(&file.path)?;
        if !mounted_path(&recipe.mount_path, &file.path).eq_ignore_ascii_case(&patch.output_path) {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "A patch critical path must identify its mounted final output.",
            ));
        }
    }
    let prefix = if recipe.mount_path.is_empty() {
        String::new()
    } else {
        format!("{}/", recipe.mount_path)
    };
    let mounted_relative = patch
        .output_path
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(&prefix))
        .and_then(|_| patch.output_path.get(prefix.len()..));
    let relative = if let Some(relative) = mounted_relative {
        relative.to_owned()
    } else {
        if recipe.auto_main_executable || !recipe.critical_files.is_empty() {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Patch output is outside its critical-file mount path.",
            ));
        }
        return Ok(Vec::new());
    };
    let file = CasFile {
        path: relative,
        blake3: patch.output_digest.value.clone(),
        size: 0,
    };
    critical_specs(&[file], &recipe.critical_files, recipe.auto_main_executable)
}

fn validate_patch_critical(layer: &CanonicalLayer, patch: &PatchSpec) -> Result<(), FusionError> {
    if patch.input_digest.algorithm != DigestAlgorithm::Blake3
        || patch.output_digest.algorithm != DigestAlgorithm::Blake3
    {
        return Err(FusionError::new(
            ErrorCode::InvalidDigest,
            "Authored patch references require BLAKE3 digests.",
        ));
    }
    for file in &layer.critical_files {
        if !mounted_path(&layer.mount_path, &file.path).eq_ignore_ascii_case(&patch.output_path)
            || !file.blake3.eq_ignore_ascii_case(&patch.output_digest.value)
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Patch critical hashes must describe the declared final output.",
            ));
        }
    }
    Ok(())
}

fn inspect_with_cache(
    cache: &LayerCache,
    file: &Path,
    options: &ArtifactInspectionOptions,
    kind: LayerKind,
) -> Result<(ArtifactInspection, CasObject), FusionError> {
    let _parent = fs_safety::guard(
        file.parent()
            .ok_or_else(|| invalid_path("Artifact has no parent."))?,
    )?;
    let handle = fs_safety::open_read(file)?;
    let size_bytes = handle
        .metadata()
        .map_err(|e| io_error("Read artifact size", file, e))?
        .len();
    let transport_digest = DigestSpec {
        algorithm: DigestAlgorithm::Blake3,
        value: hash_file(file, DigestAlgorithm::Blake3)?,
    };
    let archive = options.archive.unwrap_or_else(|| infer_archive(file));
    let raw_path = if archive == ArchiveFormat::Raw {
        Some(if kind == LayerKind::Patch {
            "patch.payload".into()
        } else {
            match &options.raw_path {
                Some(path) => path.clone(),
                None => file_name(file)?,
            }
        })
    } else {
        if options.raw_path.is_some() {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "raw_path is only valid for raw artifacts.",
            ));
        }
        None
    };
    if let Some(path) = &raw_path {
        fs_safety::validate_relative_path(path)?;
    }
    let spec = LayerSpec {
        id: "inspection".into(),
        title: None,
        description: None,
        kind,
        content: LayerContent::Generic,
        depends_on: Vec::new(),
        source: LayerSource::UserProvided {
            instructions: None,
            archive,
        },
        transport_digest: Some(transport_digest.clone()),
        size_bytes: Some(size_bytes),
        mount_path: String::new(),
        optional: false,
        default_enabled: false,
        writable_copy_paths: Vec::new(),
        critical_files: Vec::new(),
        dynamic_provider: None,
        patch: None,
    };
    // Reuse the exact LayerCache extractor and CAS preflight; there is no second
    // ZIP/7z extraction implementation in the authoring engine.
    let object = cache.object(&cache.prepare_artifact(
        &CanonicalLayer::from(spec),
        file,
        raw_path.as_deref(),
        None,
    )?)?;
    let critical_files = critical_specs(
        &object.files,
        &options.critical_files,
        options.auto_main_executable,
    )?;
    let attestation = options
        .attestation
        .as_ref()
        .map(|pin| {
            let value = SourceAttestedRelease {
                release_id: pin.release_id.clone(),
                tag: pin.release_tag.clone().ok_or_else(|| {
                    FusionError::new(
                        ErrorCode::ProviderResolutionFailed,
                        "A release attestation requires release_tag.",
                    )
                })?,
                asset_id: pin.asset_id.clone(),
                asset_url: pin.asset_url.clone(),
                transport_digest: transport_digest.clone(),
                critical_files: critical_files.clone(),
                tool_versions: pin.tool_versions.clone(),
            };
            validate_attestation(&value)?;
            Ok::<_, FusionError>(value)
        })
        .transpose()?;
    Ok((
        ArtifactInspection {
            archive,
            raw_path,
            transport_digest,
            size_bytes,
            critical_files,
            cas_digest: object.digest.clone(),
            attestation,
        },
        object,
    ))
}

fn critical_specs(
    files: &[CasFile],
    requested: &[PackerCriticalFile],
    auto_main: bool,
) -> Result<Vec<CriticalFileSpec>, FusionError> {
    let mut requested = requested.to_vec();
    if auto_main
        && !requested
            .iter()
            .any(|file| file.role == CriticalFileRole::MainExecutable)
    {
        let candidates: Vec<_> = files
            .iter()
            .filter(|file| file.path.to_ascii_lowercase().ends_with(".exe"))
            .collect();
        if candidates.len() != 1 {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "auto_main_executable requires exactly one .exe in the layer.",
            ));
        }
        requested.push(PackerCriticalFile {
            path: candidates[0].path.clone(),
            role: CriticalFileRole::MainExecutable,
        });
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for request in requested {
        fs_safety::validate_relative_path(&request.path)?;
        if !seen.insert(request.path.to_lowercase()) {
            return Err(FusionError::new(
                ErrorCode::DuplicateId,
                "Duplicate critical file path.",
            ));
        }
        let file = files
            .iter()
            .find(|file| file.path.eq_ignore_ascii_case(&request.path))
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::CorruptedCriticalBinary,
                    "A declared critical file is absent from the inspected layer.",
                )
                .with_detail("path", &request.path)
            })?;
        result.push(CriticalFileSpec {
            path: file.path.clone(),
            blake3: file.blake3.clone(),
            role: request.role,
        });
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn self_check(
    source: &str,
    text: &str,
    parents: &BTreeMap<QualifiedGameId, GameManifest>,
    target_capabilities: &[String],
) -> Result<CanonicalGameManifest, FusionError> {
    let manifest = SchemaUpcaster::upcast_game_manifest(source, text, parents)?;
    let registry = target_capabilities.iter().fold(
        CapabilityRegistry::current_client(),
        |registry, capability| registry.with(capability),
    );
    let negotiation = CapabilityNegotiator::negotiate(&manifest, &registry);
    if let Some(error) = negotiation.blockers.first() {
        return Err(error.clone());
    }
    PlanBuilder::build(&manifest, &PlanOptions::default(), &registry)?;
    let all_addons = manifest
        .layers
        .values()
        .filter(|layer| {
            layer.optional
                && matches!(
                    layer.kind,
                    LayerKind::Addon | LayerKind::Mod | LayerKind::ShaderCache
                )
        })
        .map(|layer| layer.id.clone())
        .collect();
    PlanBuilder::build(
        &manifest,
        &PlanOptions {
            enabled_addons: Some(all_addons),
            ..Default::default()
        },
        &registry,
    )?;
    if manifest.runtime.as_ref().is_some_and(|runtime| {
        manifest.layers[&runtime.layer_id]
            .dynamic_provider
            .is_some()
    }) {
        PlanBuilder::build(
            &manifest,
            &PlanOptions {
                runtime_channel: Some(RuntimeChannel::Dynamic),
                ..Default::default()
            },
            &registry,
        )?;
    }
    let context = TemplateContext {
        workspace_dir: PathBuf::from("C:/Fusion/Workspace"),
        appdata_dir: PathBuf::from("C:/Fusion/AppData"),
        local_appdata_dir: PathBuf::from("C:/Fusion/LocalAppData"),
        display_width: 1920,
        display_height: 1080,
        refresh_rate: 60,
        enabled_addons: manifest.layers.keys().cloned().collect(),
    };
    for template in &manifest.config_templates {
        ConfigTemplateEngine::render(template, &context)?;
    }
    Ok(manifest)
}

fn validate_attestation(value: &SourceAttestedRelease) -> Result<(), FusionError> {
    for field in [&value.release_id, &value.tag, &value.asset_id] {
        if field.trim().is_empty() || field.chars().any(char::is_control) {
            return Err(FusionError::new(
                ErrorCode::ProviderResolutionFailed,
                "Release identity fields must be nonempty text.",
            ));
        }
    }
    require_https(&value.asset_url)?;
    super::cas::validate_hash(&value.transport_digest.value)?;
    let mut paths = BTreeSet::new();
    for file in &value.critical_files {
        fs_safety::validate_relative_path(&file.path)?;
        super::cas::validate_hash(&file.blake3)?;
        if !paths.insert(file.path.to_lowercase()) {
            return Err(FusionError::new(
                ErrorCode::DuplicateId,
                "Release attestation critical-file paths must be unique.",
            ));
        }
    }
    if !value
        .critical_files
        .iter()
        .any(|file| file.role == CriticalFileRole::MainExecutable)
    {
        return Err(FusionError::new(
            ErrorCode::ProviderResolutionFailed,
            "A release attestation requires a hashed main executable.",
        ));
    }
    Ok(())
}

fn validate_provider_attestation(
    provider: &GithubReleaseProvider,
    value: &SourceAttestedRelease,
) -> Result<(), FusionError> {
    validate_attestation(value)?;
    if provider.tag.as_ref().is_some_and(|tag| tag != &value.tag) {
        return Err(FusionError::new(
            ErrorCode::ProviderResolutionFailed,
            "The release attestation tag differs from the provider's pinned tag.",
        ));
    }
    Ok(())
}

fn write_deterministic_zip(object: &CasObject, target: &Path) -> Result<(), FusionError> {
    let _parent = fs_safety::guard(
        target
            .parent()
            .ok_or_else(|| invalid_path("ZIP output has no parent."))?,
    )?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| io_error("Create deterministic ZIP", target, e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for entry in &object.files {
        zip.start_file(
            &entry.path,
            options.large_file(entry.size >= u32::MAX as u64),
        )
        .map_err(|e| FusionError::new(ErrorCode::ArchiveInvalid, e.to_string()))?;
        let mut input = fs_safety::open_read(
            &object
                .root
                .join(fs_safety::validate_relative_path(&entry.path)?),
        )?;
        std::io::copy(&mut input, &mut zip).map_err(|e| io_error("Stream ZIP file", target, e))?;
    }
    zip.finish()
        .and_then(|file| {
            file.sync_all()
                .map(|_| file)
                .map_err(zip::result::ZipError::Io)
        })
        .map_err(|e| FusionError::new(ErrorCode::ArchiveInvalid, e.to_string()))?;
    Ok(())
}

fn validate_recipe_names(recipe: &PackerRecipe) -> Result<(), FusionError> {
    let placeholder = SourceIndex {
        schema_version: SCHEMA_VERSION.into(),
        source_info: recipe.source_info.clone(),
        signature: None,
        games: recipe
            .games
            .iter()
            .map(|game| SourceGameEntry {
                id: game.id.clone(),
                title: game.title.clone(),
                version: game.version.clone(),
                platform: game.platform.clone(),
                cover_url: game.cover_url.clone(),
                manifest_url: "manifest.json".into(),
                manifest_blake3: "0".repeat(64),
            })
            .collect(),
    };
    SchemaUpcaster::upcast_source_index(utf8(&canonical_bytes(&placeholder)?)?)?;
    if recipe.games.is_empty() {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "A build recipe must contain at least one game.",
        ));
    }
    if let Some(base) = &recipe.base_url {
        require_https(base)?;
    }
    for game in &recipe.games {
        fs_safety::validate_relative_path(&game.id)?;
        for layer in &game.layers {
            let index = SourceIndex {
                games: vec![SourceGameEntry {
                    id: layer.id.clone(),
                    title: "Layer".into(),
                    version: "1".into(),
                    platform: None,
                    cover_url: None,
                    manifest_url: "manifest.json".into(),
                    manifest_blake3: "0".repeat(64),
                }],
                ..placeholder.clone()
            };
            SchemaUpcaster::upcast_source_index(utf8(&canonical_bytes(&index)?)?)?;
            fs_safety::validate_relative_path(&layer.id)?;
            if !layer.mount_path.is_empty() {
                fs_safety::validate_relative_path(&layer.mount_path)?;
            }
        }
    }
    Ok(())
}

fn validate_layer_inventory(
    manifest: &CanonicalGameManifest,
    inventories: &BTreeMap<(String, String), Vec<CasFile>>,
) -> Result<(), FusionError> {
    if manifest.creator.is_some() {
        let verified: Vec<ProjectionLayer> = manifest
            .layers
            .values()
            .filter_map(|layer| {
                inventories
                    .get(&(manifest.id.clone(), layer.id.clone()))
                    .map(|files| ProjectionLayer {
                        layer_id: layer.id.clone(),
                        mount_path: layer.mount_path.clone(),
                        verified_inventory: files.clone(),
                    })
            })
            .collect();
        crate::fusion_creator::profiles::resolve_declared_profiles(manifest, &verified)?;
        return Ok(());
    }
    let mut mounted: BTreeMap<String, String> = BTreeMap::new();
    let mut critical: BTreeMap<String, String> = BTreeMap::new();
    let mut inventory_segment = Vec::new();
    for id in &manifest.layer_order {
        let layer = &manifest.layers[id];
        let join = |path: &str| {
            if layer.mount_path.is_empty() {
                path.to_lowercase()
            } else {
                format!("{}/{}", layer.mount_path, path).to_lowercase()
            }
        };
        if let Some(patch) = &layer.patch {
            // A patch consumes the projection at this exact barrier. Its payload
            // is not a workspace inventory and its attested output has no known
            // size here: never invent a verified CasFile for either of them.
            flush_inventory_projection(&mut inventory_segment, &mut mounted)?;
            validate_patch_critical(layer, patch)?;
            let base = manifest.layers.get(&patch.base_layer).ok_or_else(|| {
                FusionError::new(ErrorCode::MissingLayer, "Patch base layer is absent.")
            })?;
            let base_expected = if let Some(previous) = &base.patch {
                previous
                    .output_path
                    .eq_ignore_ascii_case(&patch.input_path)
                    .then_some(previous.output_digest.value.as_str())
            } else if let Some(files) = inventories.get(&(manifest.id.clone(), base.id.clone())) {
                files
                    .iter()
                    .find(|file| {
                        mounted_path(&base.mount_path, &file.path)
                            .eq_ignore_ascii_case(&patch.input_path)
                    })
                    .map(|file| file.blake3.as_str())
            } else {
                match &base.source {
                    LayerSource::LocalLocator { locator } => locator
                        .key_files
                        .iter()
                        .chain(&base.critical_files)
                        .find(|file| {
                            mounted_path(&base.mount_path, &file.path)
                                .eq_ignore_ascii_case(&patch.input_path)
                        })
                        .map(|file| file.blake3.as_str()),
                    _ => None,
                }
            };
            if let Some(expected) = base_expected {
                if !expected.eq_ignore_ascii_case(&patch.input_digest.value) {
                    return Err(FusionError::new(
                        ErrorCode::IntegrityViolation,
                        "Patch input reference does not match its base-layer file.",
                    )
                    .with_detail("layerId", id)
                    .with_detail("path", &patch.input_path));
                }
            } else if !matches!(base.source, LayerSource::LocalLocator { .. })
                || inventories.contains_key(&(manifest.id.clone(), base.id.clone()))
            {
                return Err(FusionError::new(
                    ErrorCode::IntegrityViolation,
                    "Patch input is absent from its declared base layer.",
                )
                .with_detail("path", &patch.input_path));
            }
            if mounted
                .get(&patch.input_path.to_lowercase())
                .is_some_and(|actual| !actual.eq_ignore_ascii_case(&patch.input_digest.value))
            {
                return Err(FusionError::new(
                    ErrorCode::IntegrityViolation,
                    "Patch input reference does not match the current projected file.",
                ));
            }
            let output = patch.output_path.to_lowercase();
            if has_tree_conflict(&mounted, &output) {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Patch output has a file/directory conflict with another layer.",
                ));
            }
            // Payload bytes are tool inputs. The workspace receives only the
            // final output attested by the author-supplied reference file.
            mounted.insert(output, patch.output_digest.value.clone());
        } else if let Some(files) = inventories.get(&(manifest.id.clone(), id.clone())) {
            inventory_segment.push(ProjectionLayer {
                layer_id: id.clone(),
                mount_path: layer.mount_path.clone(),
                verified_inventory: files.clone(),
            });
        }
        let locator_keys = match &layer.source {
            LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
            _ => &[],
        };
        for file in locator_keys.iter().chain(&layer.critical_files) {
            critical.insert(join(&file.path), file.blake3.clone());
        }
    }
    flush_inventory_projection(&mut inventory_segment, &mut mounted)?;
    for (path, digest) in critical {
        if mounted.get(&path).is_some_and(|actual| actual != &digest) {
            return Err(FusionError::new(
                ErrorCode::CorruptedCriticalBinary,
                "A later layer overwrites a declared critical binary.",
            )
            .with_detail("path", &path));
        }
        if manifest
            .config_templates
            .iter()
            .any(|template| template.path.to_lowercase() == path)
        {
            return Err(FusionError::new(
                ErrorCode::InvalidTemplate,
                "A config template cannot replace a critical binary.",
            ));
        }
    }
    for template in &manifest.config_templates {
        if has_tree_conflict(&mounted, &template.path.to_lowercase()) {
            return Err(FusionError::new(
                ErrorCode::InvalidTemplate,
                "Config output has a file/directory conflict with a projected layer.",
            ));
        }
        mounted.insert(template.path.to_lowercase(), String::new());
    }
    Ok(())
}

/// Resolve ordinary inventories through the same pure resolver as the installer.
/// Patch outputs remain attested transitions between segments, not CAS providers.
fn flush_inventory_projection(
    selected_layers: &mut Vec<ProjectionLayer>,
    mounted: &mut BTreeMap<String, String>,
) -> Result<(), FusionError> {
    let projection = resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Legacy10,
        selected_layers,
        rules: &[],
    })?;
    for (path, file) in projection.files {
        // The resolver checks the segment's tree. Retain the barrier guard for
        // earlier patch outputs and inventories, which are not removed by patches.
        if has_tree_conflict(mounted, &path) {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Layer projection contains a file/directory conflict.",
            )
            .with_detail("path", &path));
        }
        mounted.insert(path, file.blake3);
    }
    selected_layers.clear();
    Ok(())
}

fn has_tree_conflict(files: &BTreeMap<String, String>, path: &str) -> bool {
    for (index, _) in path.match_indices('/') {
        if files.contains_key(&path[..index]) {
            return true;
        }
    }
    let prefix = format!("{path}/");
    files
        .range(prefix.clone()..)
        .next()
        .is_some_and(|(candidate, _)| candidate.starts_with(&prefix))
}

fn inherited_bindings(
    manifest: &CanonicalGameManifest,
    root: &Path,
    index: &SourceIndex,
    recipes: &BTreeMap<QualifiedGameId, Vec<ArtifactBinding>>,
    parents: &BTreeMap<QualifiedGameId, GameManifest>,
) -> Result<Vec<ArtifactBinding>, FusionError> {
    let mut result = BTreeMap::new();
    for key in ancestor_keys(manifest, parents)?.into_iter().rev() {
        let bindings = if let Some(bindings) = recipes.get(&key) {
            bindings.clone()
        } else if key.source_id == index.source_info.id {
            if let Some(entry) = index.games.iter().find(|entry| entry.id == key.game_id) {
                let inventory: ManifestInventory = parse_json(&read_text(
                    &root.join(inventory_relative(&entry.id, &entry.manifest_blake3)),
                )?)?;
                inventory.artifacts
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        for binding in bindings {
            result.insert(binding.layer_id.clone(), binding);
        }
    }
    Ok(result.into_values().collect())
}

fn ancestor_keys(
    manifest: &CanonicalGameManifest,
    parents: &BTreeMap<QualifiedGameId, GameManifest>,
) -> Result<Vec<QualifiedGameId>, FusionError> {
    let mut next = manifest.extends.clone();
    let mut result = Vec::new();
    while let Some(key) = next {
        if result.contains(&key) {
            return Err(FusionError::new(
                ErrorCode::ExtendsCycle,
                "Authoring ancestry contains a cycle.",
            ));
        }
        let parent = parents.get(&key).ok_or_else(|| {
            FusionError::new(
                ErrorCode::MissingParent,
                "Authoring ancestry is incomplete.",
            )
        })?;
        next = parent.extends.as_deref().map(|value| {
            if let Some((source_id, game_id)) = value.split_once("::") {
                QualifiedGameId {
                    source_id: source_id.into(),
                    game_id: game_id.into(),
                }
            } else {
                QualifiedGameId {
                    source_id: key.source_id.clone(),
                    game_id: value.into(),
                }
            }
        });
        result.push(key);
    }
    Ok(result)
}

fn inventory_local_directory(root: &Path) -> Result<Vec<CasFile>, FusionError> {
    let _root = fs_safety::guard(root)?;
    let mut result = Vec::new();
    for (relative, path) in fs_safety::list_plain_files(root)? {
        let file = fs_safety::open_read(&path)?;
        let size = file
            .metadata()
            .map_err(|e| io_error("Inspect locator file", &path, e))?
            .len();
        result.push(CasFile {
            path: relative,
            blake3: hash_file(&path, DigestAlgorithm::Blake3)?,
            size,
        });
    }
    Ok(result)
}

fn artifact_mirrors(
    layer: &PackerLayerRecipe,
    source_base: Option<&str>,
    filename: &str,
) -> Result<Vec<String>, FusionError> {
    let mut mirrors: Vec<String> = layer
        .mirrors
        .iter()
        .map(|mirror| mirror.replace("{{ARTIFACT_FILENAME}}", filename))
        .collect();
    if let Some(base) = &layer.base_url {
        mirrors.push(publication_url(Some(base), filename)?);
    }
    if mirrors.is_empty() {
        mirrors.push(publication_url(
            source_base,
            &format!("artifacts/{filename}"),
        )?);
    }
    for mirror in &mirrors {
        require_https(mirror)?;
    }
    Ok(mirrors)
}

fn publication_url(base: Option<&str>, relative: &str) -> Result<String, FusionError> {
    fs_safety::validate_relative_path(relative)?;
    match base {
        Some(base) => {
            require_https(base)?;
            let base = url::Url::parse(&format!("{}/", base.trim_end_matches('/')))
                .map_err(|e| FusionError::new(ErrorCode::InvalidUri, e.to_string()))?;
            base.join(relative)
                .map(|url| url.to_string())
                .map_err(|e| FusionError::new(ErrorCode::InvalidUri, e.to_string()))
        }
        None => Ok(relative.into()),
    }
}

fn require_https(value: &str) -> Result<(), FusionError> {
    super::adapters::https_url(value).map(|_| ())
}

fn load_index_manifests(
    root: &Path,
    index: &SourceIndex,
) -> Result<BTreeMap<QualifiedGameId, GameManifest>, FusionError> {
    let mut result = BTreeMap::new();
    for entry in &index.games {
        let path = indexed_manifest_path(root, entry)?;
        let bytes = read_bytes(&path)?;
        verify_raw_manifest(&bytes, &entry.manifest_blake3)?;
        let manifest = SchemaUpcaster::parse_game_manifest(utf8(&bytes)?)?;
        if manifest.id != entry.id
            || manifest.version != entry.version
            || manifest.title != entry.title
        {
            return Err(FusionError::new(
                ErrorCode::ManifestDigestMismatch,
                "Index metadata does not match its manifest.",
            ));
        }
        result.insert(
            QualifiedGameId {
                source_id: index.source_info.id.clone(),
                game_id: entry.id.clone(),
            },
            manifest,
        );
    }
    Ok(result)
}

fn indexed_manifest_path(root: &Path, entry: &SourceGameEntry) -> Result<PathBuf, FusionError> {
    // Packer always keeps a local immutable copy, even if the index advertises
    // an absolute publication URL. Never infer a local path from arbitrary URLs.
    let immutable = root.join(format!(
        "manifests/{}-{}.json",
        entry.id, entry.manifest_blake3
    ));
    if immutable
        .try_exists()
        .map_err(|e| io_error("Inspect manifest", &immutable, e))?
    {
        return Ok(immutable);
    }
    if !entry.manifest_url.contains(':') {
        return Ok(root.join(fs_safety::validate_relative_path(&entry.manifest_url)?));
    }
    Err(FusionError::new(
        ErrorCode::MissingParent,
        "Indexed manifest has no local repository copy.",
    )
    .with_detail("gameId", &entry.id))
}

fn verify_raw_manifest(bytes: &[u8], expected: &str) -> Result<(), FusionError> {
    if blake3::hash(bytes).to_hex().as_str() != expected {
        return Err(FusionError::new(
            ErrorCode::ManifestDigestMismatch,
            "Raw manifest BLAKE3 differs from the source index.",
        ));
    }
    Ok(())
}

fn inventory_relative(id: &str, digest: &str) -> String {
    format!("manifests/{id}-{digest}.artifacts.json")
}
fn archive_extension(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Raw => "raw",
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::SevenZip => "7z",
    }
}
fn infer_archive(path: &Path) -> ArchiveFormat {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "zip" => ArchiveFormat::Zip,
        "7z" => ArchiveFormat::SevenZip,
        _ => ArchiveFormat::Raw,
    }
}
fn file_name(path: &Path) -> Result<String, FusionError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            invalid_path("A raw artifact requires a Unicode filename or explicit raw_path.")
        })
}
fn canonical_bytes(value: &impl Serialize) -> Result<Vec<u8>, FusionError> {
    let value = serde_json::to_value(value)
        .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?;
    let mut bytes = serde_json::to_vec_pretty(&value)
        .map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn parse_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, FusionError> {
    serde_json::from_str(text).map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))
}
fn utf8(bytes: &[u8]) -> Result<&str, FusionError> {
    std::str::from_utf8(bytes).map_err(|e| FusionError::new(ErrorCode::InvalidJson, e.to_string()))
}
fn read_text(path: &Path) -> Result<String, FusionError> {
    let bytes = read_bytes(path)?;
    Ok(utf8(&bytes)?.to_owned())
}
fn read_bytes(path: &Path) -> Result<Vec<u8>, FusionError> {
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    let file = fs_safety::open_read(path)?;
    if file
        .metadata()
        .map_err(|e| io_error("Inspect JSON", path, e))?
        .len()
        > MAX_BYTES
    {
        return Err(FusionError::new(
            ErrorCode::InvalidJson,
            "Authoring JSON/TOML exceeds 16 MiB.",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("Read authoring file", path, e))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(FusionError::new(
            ErrorCode::InvalidJson,
            "Authoring JSON/TOML exceeds 16 MiB.",
        ));
    }
    Ok(bytes)
}
fn absolute(path: &Path) -> Result<PathBuf, FusionError> {
    if path.as_os_str().is_empty() {
        return Err(invalid_path(
            "An authoring filesystem path cannot be empty.",
        ));
    }
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        // Drive-relative and root-relative Windows paths depend on ambient
        // per-drive state. Authors must use an absolute or ordinary relative path.
        if path.has_root()
            || path
                .components()
                .any(|part| matches!(part, Component::Prefix(_)))
        {
            return Err(invalid_path(
                "Use an absolute or ordinary relative authoring path.",
            ));
        }
        std::env::current_dir()
            .map_err(|e| io_error("Resolve working directory", path, e))?
            .join(path)
    };
    // These are author-supplied input/output paths, not manifest-relative paths.
    // Normalize lexically without following a symlink or requiring output to exist.
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(invalid_path(
                        "Authoring path traverses above its filesystem root.",
                    ));
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    if !normalized.is_absolute() {
        return Err(invalid_path("Cannot resolve an absolute authoring path."));
    }
    Ok(normalized)
}
fn resolve_input(root: &Path, path: &Path) -> Result<PathBuf, FusionError> {
    if path.is_absolute() {
        absolute(path)
    } else {
        if path.has_root()
            || path
                .components()
                .any(|part| matches!(part, Component::Prefix(_)))
        {
            return Err(invalid_path(
                "Use an absolute or ordinary relative input path.",
            ));
        }
        if path.as_os_str().is_empty() {
            return Err(invalid_path("An input path cannot be empty."));
        }
        absolute(&root.join(path))
    }
}
fn invalid_path(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidPath, message)
}
fn reject_output_inside_source(source: &Path, out: &Path) -> Result<(), FusionError> {
    let source = source
        .canonicalize()
        .map_err(|e| io_error("Resolve source root", source, e))?;
    let out = out
        .canonicalize()
        .map_err(|e| io_error("Resolve output root", out, e))?;
    let source = source.to_string_lossy().replace('\\', "/").to_lowercase();
    let out = out.to_string_lossy().replace('\\', "/").to_lowercase();
    if out == source || out.starts_with(&format!("{}/", source.trim_end_matches('/'))) {
        return Err(invalid_path(
            "Repository output cannot be inside a layer input directory.",
        ));
    }
    Ok(())
}

fn publish_immutable_bytes(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), FusionError> {
    let target = root.join(fs_safety::validate_relative_path(relative)?);
    if target
        .try_exists()
        .map_err(|e| io_error("Inspect publication", &target, e))?
    {
        if read_bytes(&target)? != bytes {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "An immutable publication name contains different bytes.",
            ));
        }
        return Ok(());
    }
    fs_safety::atomic_write(&target, bytes)
}
fn publish_immutable_copy(root: &Path, relative: &str, source: &Path) -> Result<(), FusionError> {
    let target = root.join(fs_safety::validate_relative_path(relative)?);
    if target
        .try_exists()
        .map_err(|e| io_error("Inspect artifact publication", &target, e))?
    {
        if hash_file(&target, DigestAlgorithm::Blake3)?
            != hash_file(source, DigestAlgorithm::Blake3)?
        {
            return Err(FusionError::new(
                ErrorCode::TransportDigestMismatch,
                "Existing artifact has different bytes.",
            ));
        }
        return Ok(());
    }
    let parent = target
        .parent()
        .ok_or_else(|| invalid_path("Artifact has no parent."))?;
    let _parent = fs_safety::ensure_directory(parent)?;
    let temporary = parent.join(fs_safety::unique_name(".artifact"));
    fs_safety::copy_file(source, &temporary)?;
    // The repository writer lock serializes this no-replace publication.
    let result =
        fs::rename(&temporary, &target).map_err(|e| io_error("Publish artifact", &target, e));
    if result.is_err() {
        let _ = fs_safety::unlink_file(&temporary);
    }
    result
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, FusionError> {
        let parent = absolute(&std::env::temp_dir())?;
        Self::new_in(&parent)
    }

    /// Large archive/CAS staging stays on the author's selected repository volume.
    fn new_in(parent: &Path) -> Result<Self, FusionError> {
        let _parent = fs_safety::guard(parent)?;
        let root = parent.join(fs_safety::unique_name(".packer"));
        fs::create_dir(&root).map_err(|e| io_error("Create authoring staging", &root, e))?;
        Ok(Self(root))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs_safety::safe_remove_tree(&self.0);
    }
}

struct RepositoryLock {
    _file: File,
}
impl RepositoryLock {
    fn acquire(root: &Path) -> Result<Self, FusionError> {
        let path = root.join(".fusion-packer.lock");
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            fs_safety::check_metadata(&metadata, &path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x00200000);
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0x20000);
        }
        let file = options
            .open(&path)
            .map_err(|e| io_error("Open authoring lock", &path, e))?;
        fs_safety::check_metadata(
            &file
                .metadata()
                .map_err(|e| io_error("Inspect authoring lock", &path, e))?,
            &path,
        )?;
        if fs_safety::link_count(&file)? != 1 {
            return Err(invalid_path("Authoring lock cannot be a hardlink."));
        }
        file.try_lock_exclusive().map_err(|e| {
            FusionError::new(
                ErrorCode::OperationInProgress,
                format!("Another authoring process owns this repository: {e}"),
            )
        })?;
        Ok(Self { _file: file })
    }
}
