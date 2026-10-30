//! Normalized, version-independent contracts consumed by planning and future execution.
//!
//! This module carries no Tauri, emulator registry or filesystem dependencies. Validation
//! and inheritance resolution belong to `evolution`; execution consumes the resolved form.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::schema::{
    CapabilityFallback, CapabilityRequirements, ConfigTemplateSpec, CriticalFileSpec, DigestSpec,
    GithubReleaseProvider, LayerContent, LayerKind, LayerSource, LayerSpec, LocalPlacementPolicy,
    OrchestratorSpec, PatchSpec, PersistenceSpec, PostSetupStep, ProcessSpec, RuntimeSpec,
    SourceGameEntry, SourceInfo,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualifiedGameId {
    pub source_id: String,
    pub game_id: String,
}

impl std::fmt::Display for QualifiedGameId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}::{}", self.source_id, self.game_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalSourceIndex {
    pub schema_version: String,
    pub source_info: SourceInfo,
    pub games: Vec<SourceGameEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalGameManifest {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<super::creator_schema::CreatorManifest>,
    pub source_id: String,
    pub id: String,
    pub title: String,
    pub version: String,
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    pub extends: Option<QualifiedGameId>,
    pub capabilities: CapabilityRequirements,
    pub fallback: Vec<CapabilityFallback>,
    pub layers: BTreeMap<String, CanonicalLayer>,
    /// Stable dependency-first topological order; map iteration is not execution order.
    pub layer_order: Vec<String>,
    pub runtime: Option<RuntimeSpec>,
    pub launch: Option<ProcessSpec>,
    pub orchestrator: Option<OrchestratorSpec>,
    pub persistence: PersistenceSpec,
    pub config_templates: Vec<ConfigTemplateSpec>,
    pub post_setup_steps: Vec<PostSetupStep>,
    /// Applies to external local-locator bases. Managed CAS/workspace share library_root's volume.
    pub local_placement: LocalPlacementPolicy,
}

/// A published CAS object is immutable. A hardlink shares these read-only bytes.
/// Writable paths, patch outputs and generated configs must become separate files.
/// This applies to managed CAS objects; it never authorizes changing permissions
/// or bytes of the external installation discovered by a local locator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CasAccess {
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalLayer {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: LayerKind,
    pub content: LayerContent,
    pub depends_on: Vec<String>,
    pub source: LayerSource,
    pub transport_digest: Option<DigestSpec>,
    pub size_bytes: Option<u64>,
    pub mount_path: String,
    pub optional: bool,
    pub default_enabled: bool,
    /// Layer-relative patterns; planning resolves them beneath this layer's mount_path.
    pub writable_copy_paths: Vec<String>,
    pub critical_files: Vec<CriticalFileSpec>,
    pub dynamic_provider: Option<GithubReleaseProvider>,
    pub patch: Option<PatchSpec>,
    pub cas_access: CasAccess,
}

impl From<LayerSpec> for CanonicalLayer {
    fn from(layer: LayerSpec) -> Self {
        Self {
            id: layer.id,
            title: layer.title,
            description: layer.description,
            kind: layer.kind,
            content: layer.content,
            depends_on: layer.depends_on,
            source: layer.source,
            transport_digest: layer.transport_digest,
            size_bytes: layer.size_bytes,
            mount_path: layer.mount_path,
            optional: layer.optional,
            default_enabled: layer.default_enabled,
            writable_copy_paths: layer.writable_copy_paths,
            critical_files: layer.critical_files,
            dynamic_provider: layer.dynamic_provider,
            patch: layer.patch,
            cas_access: CasAccess::ReadOnly,
        }
    }
}

impl From<CanonicalLayer> for LayerSpec {
    fn from(layer: CanonicalLayer) -> Self {
        Self {
            id: layer.id,
            title: layer.title,
            description: layer.description,
            kind: layer.kind,
            content: layer.content,
            depends_on: layer.depends_on,
            source: layer.source,
            transport_digest: layer.transport_digest,
            size_bytes: layer.size_bytes,
            mount_path: layer.mount_path,
            optional: layer.optional,
            default_enabled: layer.default_enabled,
            writable_copy_paths: layer.writable_copy_paths,
            critical_files: layer.critical_files,
            dynamic_provider: layer.dynamic_provider,
            patch: layer.patch,
        }
    }
}
