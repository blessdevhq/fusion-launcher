//! Dedicated camelCase IPC contracts. External source JSON stays snake_case.

use super::canonical::{CanonicalGameManifest, CanonicalSourceIndex, QualifiedGameId};
use super::errors::FusionError;
use super::schema::{CriticalFileRole, RuntimeChannel};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameOptions {
    pub enabled_addons: Option<BTreeSet<String>>,
    pub runtime_channel: Option<RuntimeChannel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionSourceRecord {
    pub index: CanonicalSourceIndex,
    pub url: String,
    pub connected_at: String,
    pub refreshed_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub source_id: String,
    pub name: String,
    pub url: String,
    pub game_count: usize,
    pub public_key: Option<String>,
    pub refreshed_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreview {
    pub source_id: String,
    pub name: String,
    pub url: String,
    pub game_count: usize,
    pub public_key: Option<String>,
    pub signature_verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionCatalogRecord {
    pub source_id: String,
    pub source_name: String,
    pub entry: super::schema::SourceGameEntry,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionCatalogEntry {
    pub kind: &'static str,
    pub source_id: String,
    pub source_name: String,
    pub game_id: String,
    pub title: String,
    pub version: String,
    pub platform: Option<String>,
    pub cover_url: Option<String>,
    pub installed: bool,
    pub active_version: Option<String>,
    pub active_snapshot_id: Option<String>,
    pub ready_to_play: bool,
    pub update_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionInstallationRecord {
    pub game: QualifiedGameId,
    pub options: GameOptions,
    pub active_snapshot_id: Option<String>,
    pub library_root: Option<String>,
    pub external_workspace_root: Option<String>,
    pub active_version: Option<String>,
    pub active_manifest_blake3: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct FusionManifestRecord {
    pub game: QualifiedGameId,
    pub digest: String,
    pub raw_bytes: Vec<u8>,
    pub canonical: CanonicalGameManifest,
}

/// One local metadata read for the whole catalog; no manifest resolution or payload hashing.
#[derive(Default)]
pub struct CatalogLocalState {
    pub installations: BTreeMap<QualifiedGameId, FusionInstallationRecord>,
    pub snapshots: BTreeMap<QualifiedGameId, super::workspace::WorkspaceSnapshot>,
    pub manifests: BTreeMap<QualifiedGameId, CanonicalGameManifest>,
    pub trusted_executables: BTreeMap<QualifiedGameId, BTreeSet<(String, String)>>,
    pub active_operations: BTreeSet<QualifiedGameId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionWorkspaceRecord {
    pub id: String,
    pub game: QualifiedGameId,
    pub snapshot_json: String,
    pub manifest_blake3: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionOperationRecord {
    pub id: String,
    pub game: QualifiedGameId,
    pub kind: String,
    pub phase: String,
    pub sequence: u64,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgress {
    pub operation_id: String,
    pub source_id: String,
    pub game_id: String,
    pub layer_id: Option<String>,
    pub phase: String,
    pub sequence: u64,
    pub status: String,
    pub error: Option<FusionError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<super::adapters::TransferProgress>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonState {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub kind: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserLayerRequirement {
    pub layer_id: String,
    pub instructions: Option<String>,
    pub archive: String,
    pub raw_path_required: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableTrustState {
    pub path: String,
    pub blake3: String,
    pub role: CriticalFileRole,
    pub trusted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionGameState {
    pub source_id: String,
    pub game_id: String,
    pub title: String,
    pub version: String,
    pub platform: Option<String>,
    pub installed: bool,
    pub active_version: Option<String>,
    pub active_snapshot_id: Option<String>,
    pub rollback_available: bool,
    pub update_available: bool,
    pub ready: bool,
    pub options: GameOptions,
    pub applied_options: Option<GameOptions>,
    pub default_runtime_channel: Option<RuntimeChannel>,
    pub external_workspace_root: Option<String>,
    pub addons: Vec<AddonState>,
    pub runtime_channels: Vec<RuntimeChannel>,
    pub user_layers: Vec<UserLayerRequirement>,
    pub executables: Vec<ExecutableTrustState>,
    pub warnings: Vec<String>,
    pub blockers: Vec<FusionError>,
    pub operation: Option<OperationProgress>,
    pub crash_report_ids: Vec<String>,
    pub crash_reports: Vec<CrashSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashSummary {
    pub report_id: String,
    pub created_at: String,
    pub exit_code: Option<i64>,
    pub crash_signature_hash: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallRequest {
    #[serde(default)]
    pub user_files: BTreeMap<String, String>,
    /// Required for raw files; filenames are never inferred from untrusted URLs.
    #[serde(default)]
    pub raw_paths: BTreeMap<String, String>,
    pub external_workspace_root: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionCrashRecord {
    pub id: String,
    pub game: QualifiedGameId,
    /// Created only from SanitizedCrashReport by the storage adapter.
    pub report: Value,
    pub status: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashPreview {
    pub report_id: String,
    pub report: Value,
    pub status: String,
    pub endpoint: Option<String>,
    pub can_send: bool,
}
