//! Wave-zero DTOs and signatures. No project writes, job runner, export or input backend.
pub mod declarative;
pub mod input;
pub mod profiles;
pub(crate) mod project_storage;
pub mod projection;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::fusion_source::creator_schema::{
    AdapterReference, BuildProfile, ComponentOrigin, ControlsProfile, ProjectionRule,
};
use crate::fusion_source::errors::FusionError;
use crate::fusion_source::program_schema::{CreatorProgramProject, ProgramExecutor, MAX_REVISION};

// Nullable fields remain required on the wire. Cross-reference/IO policy is C2's.
fn required_nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

fn wire_revision<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let value = u64::deserialize(d)?;
    if value > MAX_REVISION {
        return Err(serde::de::Error::custom(
            "Creator revision is outside the wire range.",
        ));
    }
    Ok(value)
}

fn expected_revision<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let value = Option::<u64>::deserialize(d)?;
    if value.is_some_and(|value| value > MAX_REVISION) {
        return Err(serde::de::Error::custom(
            "Creator revision is outside the wire range.",
        ));
    }
    Ok(value)
}

pub const JOB_PROGRESS_EVENT: &str = "fusion-creator:job-progress";
pub const CREATOR_COMMANDS: &[&str] = &[
    "fusion_creator_project_list",
    "fusion_creator_project_open",
    "fusion_creator_project_save",
    "fusion_creator_scan",
    "fusion_creator_classify",
    "fusion_creator_build",
    "fusion_creator_verify",
    "fusion_creator_export",
    "fusion_creator_verify_https",
    "fusion_creator_job_get",
    "fusion_creator_job_cancel",
    "fusion_creator_job_resume",
    "fusion_creator_release_list",
    "fusion_creator_report_get",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatorComponent {
    pub id: String,
    pub layer_id: String,
    pub origin: ComponentOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Legacy validation/compatibility tests only; never a project IPC/storage carrier.
pub struct CreatorProject {
    pub schema_version: String,
    pub project_id: String,
    pub revision: u64,
    pub title: String,
    pub source_id: String,
    pub game_id: String,
    pub game_version: String,
    pub adapter: AdapterReference,
    pub components: Vec<CreatorComponent>,
    pub build_profiles: Vec<BuildProfile>,
    pub controls_profiles: Vec<ControlsProfile>,
    pub projection_rules: Vec<ProjectionRule>,
}

/// One finite build/control pairing. Installing it is owner B's implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorProfileSelection {
    pub build_profile_id: String,
    pub controls_profile_id: String,
}

/// Private local IPC/storage only; never serialized into a portable project or Source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorPrivateBindings {
    pub project_id: String,
    pub component_paths: BTreeMap<String, String>,
    #[serde(deserialize_with = "required_nullable")]
    pub signing_key_ref: Option<String>,
}

/// Shared with project_storage::open_private_bindings; revision is independent of the project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedPrivateBindings {
    #[serde(deserialize_with = "wire_revision")]
    pub revision: u64,
    pub bindings: CreatorPrivateBindings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorProjectOpenResponse {
    pub project: CreatorProgramProject,
    #[serde(deserialize_with = "required_nullable")]
    pub private_bindings: Option<SavedPrivateBindings>,
    /// Component IDs in program_descriptor that need local selection/rebinding.
    pub missing_inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorProjectSummary {
    pub project_id: String,
    pub title: String,
    #[serde(deserialize_with = "wire_revision")]
    pub revision: u64,
    pub verification_status: CreatorReportStatus,
    /// UTC Unix epoch milliseconds, bounded to a safe JSON integer.
    #[serde(deserialize_with = "wire_revision")]
    pub modified_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatorJobKind {
    Scan,
    Classify,
    Build,
    Verify,
    Export,
    VerifyHttps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatorJobStatus {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Interrupted,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorJobProgress {
    pub job_id: String,
    pub project_id: String,
    pub project_revision: u64,
    /// Strictly increasing per job, persisted across restart; 1..=2^53-1.
    pub sequence: u64,
    pub kind: CreatorJobKind,
    pub status: CreatorJobStatus,
    pub phase: String,
    pub completed_units: u64,
    pub total_units: Option<u64>,
    pub checkpoint_id: Option<String>,
    pub release_id: Option<String>,
    pub error: Option<FusionError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorJobReceipt {
    pub job_id: String,
    pub project_id: String,
    pub project_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatorCheckpointBoundary {
    ScanClassificationSaved,
    ArtifactCommitted,
    ReleaseBuilt,
    VerificationCompleted,
    ExportCompleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorResumeStamp {
    pub project_revision: u64,
    pub input_hashes: BTreeMap<String, String>,
    pub executor: ProgramExecutor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorCheckpoint {
    pub checkpoint_id: String,
    pub job_id: String,
    pub boundary: CreatorCheckpointBoundary,
    pub stamp: CreatorResumeStamp,
    /// Hash of the verified, already atomically committed result, not a partial file.
    pub result_blake3: String,
    pub artifact_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorJob {
    pub progress: CreatorJobProgress,
    pub checkpoints: Vec<CreatorCheckpoint>,
    pub report_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatorReportKind {
    StructureIntegrity,
    CleanInstall,
    ObservedLaunch,
    ManualScenario,
    HttpsAcquisition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatorReportStatus {
    Passed,
    Failed,
    NotTested,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorReport {
    pub report_id: String,
    pub release_id: String,
    pub release_blake3: String,
    pub project_revision: u64,
    pub kind: CreatorReportKind,
    pub status: CreatorReportStatus,
    pub executor: ProgramExecutor,
    pub build_profile_id: String,
    pub controls_profile_id: String,
    pub environment: BTreeMap<String, String>,
    pub diagnostics: Vec<FusionError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorRelease {
    pub release_id: String,
    pub project_id: String,
    pub project_revision: u64,
    pub manifest_blake3: String,
    pub artifact_hashes: BTreeMap<String, String>,
    pub report_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorProjectRequest {
    pub project_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorProjectSaveRequest {
    pub project: CreatorProgramProject,
    /// Required: null is insert-only; n requires supplied project.revision = n + 1.
    #[serde(deserialize_with = "expected_revision")]
    pub expected_revision: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorProjectSaveResponse {
    #[serde(deserialize_with = "wire_revision")]
    pub revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorJobStartRequest {
    pub project_id: String,
    pub project_revision: u64,
    pub build_profile_ids: Vec<String>,
    pub controls_profile_ids: Vec<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub release_id: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub target_directory: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub source_url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorJobRequest {
    pub job_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorReportRequest {
    pub report_id: String,
}

#[cfg(test)]
mod ipc_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_ipc_rejects_legacy_save_without_implicit_migration() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
        let request = serde_json::json!({
            "project": fixture["legacyProject"], "expectedRevision": 1
        });
        assert!(serde_json::from_value::<CreatorProjectSaveRequest>(request).is_err());
    }

    #[test]
    fn shared_wire_fixtures_round_trip_without_private_paths() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
        let project: CreatorProgramProject =
            serde_json::from_value(fixture["project"].clone()).unwrap();
        let progress: CreatorJobProgress = serde_json::from_value(fixture["job"].clone()).unwrap();
        let input: input::InputState = serde_json::from_value(fixture["input"].clone()).unwrap();
        assert_eq!(serde_json::to_value(project).unwrap(), fixture["project"]);
        assert_eq!(serde_json::to_value(progress).unwrap(), fixture["job"]);
        let selection: CreatorProfileSelection =
            serde_json::from_value(fixture["profileSelection"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(selection).unwrap(),
            fixture["profileSelection"]
        );
        let extension: crate::fusion_source::creator_schema::CreatorManifest =
            serde_json::from_value(fixture["manifest"]["creator"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(extension).unwrap(),
            fixture["manifest"]["creator"]
        );
        // JSON has one number domain; serde_json::Value distinguishes 0 from 0.0.
        // Compare the typed f64 wire round-trip, not an irrelevant lexical form.
        assert_eq!(
            serde_json::from_value::<input::InputState>(serde_json::to_value(&input).unwrap())
                .unwrap(),
            input
        );
        let mut private = fixture["project"].clone();
        private["program_descriptor"]["components"][0]["localPath"] =
            serde_json::json!("C:/private/runtime");
        assert!(serde_json::from_value::<CreatorProgramProject>(private).is_err());
        let legacy: CreatorProject =
            serde_json::from_value(fixture["legacyProject"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            fixture["legacyProject"]
        );
    }
}
