//! Wave-zero DTOs and signatures. No project writes, job runner, export or input backend.
pub mod input;
pub mod projection;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::fusion_source::creator_schema::{
    AdapterReference, BuildProfile, ComponentOrigin, ControlsProfile, ProjectionRule,
};
use crate::fusion_source::errors::FusionError;

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

/// Private local storage only; never serialized into a portable project or Source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorPrivateBindings {
    pub project_id: String,
    pub component_paths: BTreeMap<String, String>,
    pub signing_key_ref: Option<String>,
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
    pub adapter: AdapterReference,
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
    pub adapter: AdapterReference,
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
    pub project: CreatorProject,
    pub expected_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatorJobStartRequest {
    pub project_id: String,
    pub project_revision: u64,
    pub build_profile_ids: Vec<String>,
    pub controls_profile_ids: Vec<String>,
    pub release_id: Option<String>,
    pub target_directory: Option<String>,
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
mod tests {
    use super::*;
    #[test]
    fn shared_wire_fixtures_round_trip_without_private_paths() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
        let project: CreatorProject = serde_json::from_value(fixture["project"].clone()).unwrap();
        let progress: CreatorJobProgress = serde_json::from_value(fixture["job"].clone()).unwrap();
        let input: input::InputState = serde_json::from_value(fixture["input"].clone()).unwrap();
        assert_eq!(serde_json::to_value(project).unwrap(), fixture["project"]);
        assert_eq!(serde_json::to_value(progress).unwrap(), fixture["job"]);
        // JSON has one number domain; serde_json::Value distinguishes 0 from 0.0.
        // Compare the typed f64 wire round-trip, not an irrelevant lexical form.
        assert_eq!(
            serde_json::from_value::<input::InputState>(serde_json::to_value(&input).unwrap())
                .unwrap(),
            input
        );
        let mut private = fixture["project"].clone();
        private["components"][0]["localPath"] = serde_json::json!("C:/private/runtime");
        assert!(serde_json::from_value::<CreatorProject>(private).is_err());
    }
}
