//! Project IPC services over the authoritative typed project store.
//! The job runner initializes schema v7 before these services are called.
use super::declarative::compiler::validate_descriptor_metadata;
use super::project_storage;
use super::{
    CreatorProjectOpenResponse, CreatorProjectSaveRequest, CreatorProjectSaveResponse,
    CreatorProjectSummary, CreatorRelease, CreatorReport, CreatorReportKind, CreatorReportStatus,
};
use crate::fusion_source::creator_schema::ComponentOrigin;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use crate::fusion_source::program_schema::{
    parse_unique_json, CreatorProgramProject, MAX_PROGRAM_BYTES, MAX_REVISION,
};
use crate::storage::RepositoryStore;
use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;
use std::path::Path;

fn invalid_stored() -> FusionError {
    FusionError::new(
        ErrorCode::IntegrityViolation,
        "Stored Creator metadata is inconsistent.",
    )
}

fn sql_error(_: rusqlite::Error) -> FusionError {
    FusionError::new(ErrorCode::Io, "Creator storage operation failed.")
}

/// Structural/semantic metadata checks only: saving is not acquisition or build.
pub(crate) fn save(
    store: &mut RepositoryStore,
    request: CreatorProjectSaveRequest,
) -> Result<CreatorProjectSaveResponse, FusionError> {
    if request.expected_revision.is_some_and(|n| n > MAX_REVISION) {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Creator revision is outside the wire range.",
        ));
    }
    request
        .project
        .validate()
        .and_then(|()| validate_descriptor_metadata(&request.project.program_descriptor))
        .map_err(|_| {
            FusionError::new(
                ErrorCode::InvalidManifest,
                "Invalid Creator project metadata.",
            )
        })?;
    let bytes = serde_json::to_vec(&request.project).map_err(|_| {
        FusionError::new(
            ErrorCode::InvalidManifest,
            "Invalid Creator project metadata.",
        )
    })?;
    // Storage owns the immediate transaction, CAS and the existing private row.
    // v7 triggers write modified_at in that same transaction.
    let project = project_storage::save_project_bytes(store, &bytes, request.expected_revision)?;
    Ok(CreatorProjectSaveResponse {
        revision: project.revision,
    })
}

pub(crate) fn open(
    store: &RepositoryStore,
    project_id: &str,
) -> Result<CreatorProjectOpenResponse, FusionError> {
    let snapshot = store.conn.unchecked_transaction().map_err(sql_error)?;
    let project = project_storage::open_project(store, project_id)?.ok_or_else(|| {
        FusionError::new(
            ErrorCode::SnapshotNotFound,
            "Creator project was not found.",
        )
    })?;
    validate_stored_project(&project)?;
    let private_bindings = project_storage::open_private_bindings(store, project_id)?;
    if private_bindings.as_ref().is_some_and(|saved| {
        saved.bindings.component_paths.keys().any(|id| {
            !project
                .program_descriptor
                .components
                .iter()
                .any(|component| &component.id == id)
        })
    }) {
        return Err(invalid_stored());
    }
    let mut missing_inputs = Vec::new();
    for component in &project.program_descriptor.components {
        let available = private_bindings
            .as_ref()
            .and_then(|saved| saved.bindings.component_paths.get(&component.id))
            .is_some_and(|path| local_input_available(Path::new(path)));
        if available {
            continue;
        }
        match &component.origin {
            ComponentOrigin::UserProvided { .. } => missing_inputs.push(component.id.clone()),
            // No artifact/cache resolver is installed in this service. A missing
            // private row is not evidence that a managed or external input is missing.
            ComponentOrigin::Managed { .. } | ComponentOrigin::External { .. } => {
                return Err(FusionError::new(
                    ErrorCode::ActionUnavailable,
                    "Creator input availability cannot currently be resolved.",
                )
                .with_detail("command", "fusion_creator_project_open"));
            }
        }
    }
    snapshot.commit().map_err(sql_error)?;
    Ok(CreatorProjectOpenResponse {
        project,
        private_bindings,
        missing_inputs,
    })
}

/// A point-in-time availability check, not a pin check or reusable IO approval.
/// Every future acquisition must independently reopen and verify its own handle.
fn local_input_available(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if fs_safety::check_metadata(&metadata, path).is_err() {
        return false;
    }
    if metadata.is_dir() {
        fs_safety::guard(path).is_ok()
    } else {
        fs_safety::open_read(path).is_ok()
    }
}

/// Return all bounded storage pages under one read snapshot, in project-ID order.
pub(crate) fn list(store: &RepositoryStore) -> Result<Vec<CreatorProjectSummary>, FusionError> {
    let snapshot = store.conn.unchecked_transaction().map_err(sql_error)?;
    let mut summaries = Vec::new();
    let mut offset = 0_u32;
    loop {
        let projects = project_storage::list_projects(store, 32, offset)?;
        let last_page = projects.len() < 32;
        for project in projects {
            validate_stored_project(&project)?;
            let modified_at = modified_at(store, &project.project_id)?;
            let verification_status = verification_status(store, &project)?;
            summaries.push(CreatorProjectSummary {
                project_id: project.project_id,
                title: project.title,
                revision: project.revision,
                verification_status,
                modified_at,
            });
        }
        if last_page {
            break;
        }
        offset = offset.checked_add(32).ok_or_else(invalid_stored)?;
    }
    snapshot.commit().map_err(sql_error)?;
    Ok(summaries)
}

fn validate_stored_project(project: &CreatorProgramProject) -> Result<(), FusionError> {
    validate_descriptor_metadata(&project.program_descriptor).map_err(|_| invalid_stored())
}

fn modified_at(store: &RepositoryStore, project_id: &str) -> Result<u64, FusionError> {
    let mut statement = store
        .conn
        .prepare("SELECT modified_at FROM creator_project_metadata WHERE project_id=?1")
        .map_err(sql_error)?;
    let mut rows = statement.query([project_id]).map_err(sql_error)?;
    let row = rows.next().map_err(sql_error)?.ok_or_else(invalid_stored)?;
    let value: i64 = row.get(0).map_err(|_| invalid_stored())?;
    if value < 0 || value as u64 > MAX_REVISION {
        return Err(invalid_stored());
    }
    Ok(value as u64)
}

fn stored_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, FusionError> {
    let json = std::str::from_utf8(bytes).map_err(|_| invalid_stored())?;
    let value = parse_unique_json(json, MAX_PROGRAM_BYTES).map_err(|_| invalid_stored())?;
    serde_json::from_value(value).map_err(|_| invalid_stored())
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn verification_status(
    store: &RepositoryStore,
    project: &CreatorProgramProject,
) -> Result<CreatorReportStatus, FusionError> {
    let mut statement = store.conn.prepare(
        "SELECT report_id,report_json FROM creator_reports WHERE project_id=?1 AND project_revision=?2 ORDER BY report_id",
    ).map_err(sql_error)?;
    let mut rows = statement
        .query(params![project.project_id, project.revision])
        .map_err(sql_error)?;
    let mut status = CreatorReportStatus::NotTested;
    while let Some(row) = rows.next().map_err(sql_error)? {
        let id: String = row.get(0).map_err(|_| invalid_stored())?;
        let bytes: Vec<u8> = row.get(1).map_err(|_| invalid_stored())?;
        let report: CreatorReport = stored_json(&bytes)?;
        if report.report_id != id || report.project_revision != project.revision {
            return Err(invalid_stored());
        }
        validate_report_release(store, project, &report)?;
        if report.kind == CreatorReportKind::StructureIntegrity {
            match report.status {
                CreatorReportStatus::Failed => status = CreatorReportStatus::Failed,
                CreatorReportStatus::Passed if status != CreatorReportStatus::Failed => {
                    status = CreatorReportStatus::Passed;
                }
                _ => {}
            }
        }
    }
    Ok(status)
}

fn validate_report_release(
    store: &RepositoryStore,
    project: &CreatorProgramProject,
    report: &CreatorReport,
) -> Result<(), FusionError> {
    let descriptor = &project.program_descriptor;
    let allowed_pair = descriptor.build_profiles.iter().any(|build| {
        build.id == report.build_profile_id
            && build
                .controls_profile_ids
                .contains(&report.controls_profile_id)
    }) && descriptor
        .controls_profiles
        .iter()
        .any(|controls| controls.id() == report.controls_profile_id);
    if report.executor != descriptor.executor
        || !allowed_pair
        || !valid_hash(&report.release_blake3)
    {
        return Err(invalid_stored());
    }
    let row: Option<(String, i64, Vec<u8>)> = store.conn.query_row(
        "SELECT project_id,project_revision,release_json FROM creator_releases WHERE release_id=?1",
        [&report.release_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional().map_err(sql_error)?;
    let (project_id, revision, bytes) = row.ok_or_else(invalid_stored)?;
    let release: CreatorRelease = stored_json(&bytes)?;
    if project_id != project.project_id
        || revision < 0
        || revision as u64 != project.revision
        || release.release_id != report.release_id
        || release.project_id != project.project_id
        || release.project_revision != project.revision
        || release.manifest_blake3 != report.release_blake3
        || !release.report_ids.contains(&report.report_id)
        || release
            .artifact_hashes
            .values()
            .any(|hash| !valid_hash(hash))
    {
        return Err(invalid_stored());
    }
    Ok(())
}
