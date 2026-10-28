//! Fusion Source Standard v1.0 wire contracts. All paths are untrusted until validated.
//!
//! These data types describe work; deserializing them never fetches, writes or executes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIndex {
    pub schema_version: String,
    pub source_info: SourceInfo,
    pub games: Vec<SourceGameEntry>,
    /// Base64-encoded Ed25519 signature over the original canonical wire JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub maintainer: Option<String>,
    #[serde(default)]
    pub telemetry_endpoint: Option<String>,
    /// Base64-encoded 32-byte Ed25519 public key, pinned when first connected.
    #[serde(default)]
    pub public_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGameEntry {
    pub id: String,
    pub title: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    pub manifest_url: String,
    /// BLAKE3 of the exact downloaded manifest bytes, before parsing or normalization.
    pub manifest_blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameManifest {
    pub schema_version: String,
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
    pub layers: Vec<LayerSpec>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirements {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityFallback {
    pub capability: String,
    pub strategy: FallbackStrategy,
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackStrategy {
    HardlinkProjection,
    BuiltinConfigTemplates,
    SkipOptional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
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
    pub source: LayerSource,
    /// Digest of the acquired container. It does not attest unpacked executable files.
    #[serde(default)]
    pub transport_digest: Option<DigestSpec>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub mount_path: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub default_enabled: bool,
    /// Layer-relative paths/globs materialized as writable physical copies under mount_path.
    /// Other CAS files remain read-only, including through a hardlink projection.
    #[serde(default)]
    pub writable_copy_paths: Vec<String>,
    #[serde(default)]
    pub critical_files: Vec<CriticalFileSpec>,
    #[serde(default)]
    pub dynamic_provider: Option<GithubReleaseProvider>,
    #[serde(default)]
    pub patch: Option<PatchSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Base,
    SharedDeps,
    Runtime,
    Addon,
    Mod,
    ShaderCache,
    Patch,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerContent {
    #[default]
    Generic,
    Bios,
    Firmware,
    Keys,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerSource {
    Remote {
        mirrors: Vec<String>,
        archive: ArchiveFormat,
    },
    UserProvided {
        #[serde(default)]
        instructions: Option<String>,
        /// Imports can be containers too (for example, a user-owned firmware ZIP).
        #[serde(default)]
        archive: ArchiveFormat,
    },
    LocalLocator {
        locator: LocalLocatorSpec,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    #[default]
    Raw,
    Zip,
    SevenZip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestSpec {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestAlgorithm {
    Blake3,
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriticalFileSpec {
    pub path: String,
    pub blake3: String,
    pub role: CriticalFileRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriticalFileRole {
    MainExecutable,
    InjectorDll,
    CoreLibrary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubReleaseProvider {
    pub repo: String,
    pub asset_mask: String,
    pub channel: ReleaseChannel,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    Latest,
    Nightly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalLocatorSpec {
    pub store: LocalStore,
    #[serde(default)]
    pub app_id: Option<String>,
    /// Discovery hints may identify absolute external installations. They are never write targets.
    #[serde(default)]
    pub path_hints: Vec<String>,
    #[serde(default)]
    pub key_files: Vec<CriticalFileSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalStore {
    Steam,
    Gog,
    Epic,
    Manual,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalPlacementPolicy {
    /// A locator on another volume requires an explicit alternate policy.
    #[default]
    RequireSameVolume,
    /// Allocate the isolated workspace on the discovered base's volume.
    WorkspaceOnBaseVolume,
    /// Explicitly authorize a cross-volume projection fallback in the build plan.
    CrossVolumeFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionStrategy {
    Hardlink,
    CopyMutable,
    CrossVolumeFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSpec {
    pub format: PatchFormat,
    pub base_layer: String,
    pub input_path: String,
    pub output_path: String,
    pub input_digest: DigestSpec,
    pub output_digest: DigestSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchFormat {
    Xdelta3,
    Bsdiff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSpec {
    pub layer_id: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub default_channel: RuntimeChannel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeChannel {
    #[default]
    Pinned,
    Dynamic,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistenceSpec {
    #[serde(default)]
    pub saves_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigTemplateSpec {
    pub id: String,
    pub path: String,
    pub format: ConfigFormat,
    pub template: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFormat {
    Toml,
    Ini,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpec {
    pub id: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub role: ProcessRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessRole {
    Primary,
    Supporting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrchestratorSpec {
    pub processes: Vec<ProcessSpec>,
    #[serde(default)]
    pub strict_sync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PostSetupStep {
    WasmTransform {
        id: String,
        layer_id: String,
        input_path: String,
        output_path: String,
        #[serde(default)]
        optional: bool,
        /// ID of a built-in config template, selected only through capability negotiation.
        #[serde(default)]
        fallback_template: Option<String>,
    },
}
