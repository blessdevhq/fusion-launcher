//! Service tests use isolated real SQLite stores and explicitly synthetic local files.
use super::project_service;
use super::project_storage::{self, PrivateBindingsSave};
use super::{
    CreatorPrivateBindings, CreatorProjectSaveRequest, CreatorRelease, CreatorReport,
    CreatorReportKind, CreatorReportStatus,
};
use crate::fusion_source::creator_schema::ComponentOrigin;
use crate::fusion_source::errors::ErrorCode;
use crate::fusion_source::program_schema::{CreatorProgramProject, MAX_REVISION};
use crate::storage::RepositoryStore;
use rusqlite::params;
use std::collections::BTreeMap;
use std::path::Path;

fn open_store(path: &Path) -> Result<RepositoryStore, String> {
    let store = RepositoryStore::open(path)?;
    super::jobs_schema::migrate(&store.conn).map_err(|error| error.to_string())?;
    Ok(store)
}

fn project(id: &str, revision: u64) -> CreatorProgramProject {
    let descriptor: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/creator/programs/supertuxkart-1.5.json"
    ))
    .unwrap();
    let mut project = CreatorProgramProject::read_json(
        &serde_json::json!({
            "schema_version": "1.1", "project_id": id, "revision": revision,
            "title": "Service fixture", "source_id": "source", "game_id": "game",
            "game_version": "1.5", "program_descriptor": descriptor
        })
        .to_string(),
    )
    .unwrap();
    project.program_descriptor.components[0].origin = ComponentOrigin::UserProvided {
        requirement_id: "local-runtime".into(),
        inventory_blake3: "a".repeat(64),
        instructions: "Select the synthetic local runtime fixture.".into(),
    };
    project
}

fn save(store: &mut RepositoryStore, project: CreatorProgramProject, expected: Option<u64>) {
    let revision = project.revision;
    let saved = project_service::save(
        store,
        CreatorProjectSaveRequest {
            project,
            expected_revision: expected,
        },
    )
    .unwrap();
    assert_eq!(saved.revision, revision);
}

fn save_with_path(store: &mut RepositoryStore, project: &CreatorProgramProject, path: &Path) {
    let bindings = CreatorPrivateBindings {
        project_id: project.project_id.clone(),
        component_paths: BTreeMap::from([(
            project.program_descriptor.components[0].id.clone(),
            path.to_string_lossy().into_owned(),
        )]),
        signing_key_ref: Some("synthetic-private-key-reference".into()),
    };
    project_storage::save_project_with_bindings(
        store,
        &serde_json::to_vec(project).unwrap(),
        None,
        Some(&PrivateBindingsSave {
            revision: 7,
            expected_revision: None,
            bindings: &bindings,
        }),
    )
    .unwrap();
}

fn evidence(
    store: &RepositoryStore,
    project: &CreatorProgramProject,
    id: &str,
    kind: CreatorReportKind,
    status: CreatorReportStatus,
) {
    let release_id = format!("release-{id}");
    let release = CreatorRelease {
        release_id: release_id.clone(),
        project_id: project.project_id.clone(),
        project_revision: project.revision,
        manifest_blake3: "b".repeat(64),
        artifact_hashes: BTreeMap::new(),
        report_ids: vec![id.into()],
    };
    let report = CreatorReport {
        report_id: id.into(),
        release_id,
        release_blake3: release.manifest_blake3.clone(),
        project_revision: project.revision,
        kind,
        status,
        executor: project.program_descriptor.executor.clone(),
        build_profile_id: "windowed".into(),
        controls_profile_id: "native".into(),
        environment: BTreeMap::from([("os".into(), "windows".into())]),
        diagnostics: vec![],
    };
    store.conn.execute(
        "INSERT INTO creator_releases(release_id,project_id,project_revision,release_json) VALUES(?1,?2,?3,?4)",
        params![release.release_id, release.project_id, release.project_revision, serde_json::to_vec(&release).unwrap()],
    ).unwrap();
    store.conn.execute(
        "INSERT INTO creator_reports(report_id,project_id,project_revision,report_json) VALUES(?1,?2,?3,?4)",
        params![report.report_id, project.project_id, project.revision, serde_json::to_vec(&report).unwrap()],
    ).unwrap();
}

#[test]
fn service_save_persists_insert_zero_and_cas_without_changing_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let mut store = open_store(&db).unwrap();
    let initial = project("zero", 0);
    save_with_path(&mut store, &initial, dir.path());
    let before = project_storage::open_private_bindings(&store, "zero").unwrap();
    save(&mut store, project("zero", 1), Some(0));
    for expected in [None, Some(0), Some(2)] {
        let error = project_service::save(
            &mut store,
            CreatorProjectSaveRequest {
                project: project("zero", 2),
                expected_revision: expected,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::SnapshotConflict);
    }
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(project_storage::open_private_bindings(&store, "zero").unwrap())
            .unwrap()
    );
    save(&mut store, project("insert-zero", 0), None);
    save(&mut store, project("insert-seven", 7), None);
    drop(store);
    let store = open_store(&db).unwrap();
    assert_eq!(
        project_service::open(&store, "zero")
            .unwrap()
            .project
            .revision,
        1
    );
    assert_eq!(
        project_storage::open_project(&store, "insert-zero")
            .unwrap()
            .unwrap()
            .revision,
        0
    );
}

#[test]
fn service_save_uses_semantic_validation_and_rolls_back_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    save(&mut store, project("project", 0), None);
    let mut invalid = project("project", 1);
    invalid.program_descriptor.build_profiles[0].layer_ids = vec!["unknown-layer".into()];
    assert!(project_service::save(
        &mut store,
        CreatorProjectSaveRequest {
            project: invalid,
            expected_revision: Some(0),
        }
    )
    .is_err());
    assert_eq!(
        project_storage::open_project(&store, "project")
            .unwrap()
            .unwrap()
            .revision,
        0
    );
}

#[test]
fn service_list_returns_all_pages_in_stable_order_and_safe_summary_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    assert!(project_service::list(&store).unwrap().is_empty());
    for index in (0..35).rev() {
        save(&mut store, project(&format!("project-{index:02}"), 0), None);
    }
    let rows = project_service::list(&store).unwrap();
    assert_eq!(rows.len(), 35);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.project_id, format!("project-{index:02}"));
        assert_eq!(row.verification_status, CreatorReportStatus::NotTested);
        let stored: u64 = store
            .conn
            .query_row(
                "SELECT modified_at FROM creator_project_metadata WHERE project_id=?1",
                [&row.project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(row.modified_at, stored);
        assert!(row.modified_at > 0 && row.modified_at <= MAX_REVISION);
        let json = serde_json::to_value(row).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 5);
        assert!(json.get("privateBindings").is_none());
        assert!(json.get("program_descriptor").is_none());
    }
}

#[test]
fn service_list_aggregates_structure_only_and_failed_wins_for_current_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    let initial = project("project", 0);
    save(&mut store, initial.clone(), None);
    evidence(
        &store,
        &initial,
        "observed",
        CreatorReportKind::ObservedLaunch,
        CreatorReportStatus::Passed,
    );
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::NotTested
    );
    evidence(
        &store,
        &initial,
        "structure-pass",
        CreatorReportKind::StructureIntegrity,
        CreatorReportStatus::Passed,
    );
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::Passed
    );
    evidence(
        &store,
        &initial,
        "manual-fail",
        CreatorReportKind::ManualScenario,
        CreatorReportStatus::Failed,
    );
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::Passed
    );
    evidence(
        &store,
        &initial,
        "structure-fail",
        CreatorReportKind::StructureIntegrity,
        CreatorReportStatus::Failed,
    );
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::Failed
    );
    save(&mut store, project("project", 1), Some(0));
    assert_eq!(
        project_service::list(&store).unwrap()[0].verification_status,
        CreatorReportStatus::NotTested
    );
}

#[test]
fn service_list_never_borrows_another_projects_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    let other = project("other", 0);
    save(&mut store, other.clone(), None);
    save(&mut store, project("target", 0), None);
    evidence(
        &store,
        &other,
        "other-report",
        CreatorReportKind::StructureIntegrity,
        CreatorReportStatus::Passed,
    );
    let rows = project_service::list(&store).unwrap();
    assert_eq!(rows[0].verification_status, CreatorReportStatus::Passed);
    assert_eq!(rows[1].verification_status, CreatorReportStatus::NotTested);
}

#[test]
fn service_list_rejects_missing_or_out_of_range_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    save(&mut store, project("project", 0), None);
    store
        .conn
        .execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE creator_project_metadata SET modified_at=?1",
            [MAX_REVISION + 1],
        )
        .unwrap();
    assert_eq!(
        project_service::list(&store).unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
    store
        .conn
        .execute("DELETE FROM creator_project_metadata", [])
        .unwrap();
    assert_eq!(
        project_service::list(&store).unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
}

#[test]
fn service_list_rejects_corrupt_current_report_without_private_error_details() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    save(&mut store, project("project", 0), None);
    store.conn.execute(
        "INSERT INTO creator_reports(report_id,project_id,project_revision,report_json) VALUES('bad','project',0,?1)",
        [b"synthetic-private-path-and-credential".as_slice()],
    ).unwrap();
    let error = project_service::list(&store).unwrap_err();
    assert_eq!(error.code, ErrorCode::IntegrityViolation);
    assert!(!serde_json::to_string(&error)
        .unwrap()
        .contains("synthetic-private"));
}

#[test]
fn service_open_missing_user_input_reports_component_ids_not_paths() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    let initial = project("project", 0);
    save(&mut store, initial.clone(), None);
    let opened = project_service::open(&store, "project").unwrap();
    assert!(opened.private_bindings.is_none());
    assert_eq!(
        opened.missing_inputs,
        vec![initial.program_descriptor.components[0].id.clone()]
    );
}

#[test]
fn service_open_existing_file_or_directory_needs_no_rebinding_but_does_not_verify_pins() {
    for file_input in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(&dir.path().join("library.db")).unwrap();
        let input = dir.path().join("synthetic-private-input");
        if file_input {
            std::fs::write(&input, b"not a pinned runtime").unwrap();
        } else {
            std::fs::create_dir(&input).unwrap();
        }
        let initial = project("project", 0);
        save_with_path(&mut store, &initial, &input);
        let opened = project_service::open(&store, "project").unwrap();
        assert!(opened.missing_inputs.is_empty());
        assert_eq!(opened.private_bindings.unwrap().revision, 7);
        assert_eq!(
            project_service::list(&store).unwrap()[0].verification_status,
            CreatorReportStatus::NotTested
        );
        let portable = serde_json::to_string(&opened.project).unwrap();
        assert!(!portable.contains("synthetic-private"));
    }
}

#[test]
fn service_open_disappeared_or_relative_user_binding_needs_rebinding() {
    for relative in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(&dir.path().join("library.db")).unwrap();
        let initial = project("project", 0);
        let missing = dir.path().join("synthetic-disappeared-input");
        let path = if relative {
            Path::new("relative-input")
        } else {
            &missing
        };
        save_with_path(&mut store, &initial, path);
        let opened = project_service::open(&store, "project").unwrap();
        assert_eq!(
            opened.missing_inputs,
            vec![initial.program_descriptor.components[0].id.clone()]
        );
        assert!(opened.private_bindings.is_some());
    }
}

#[test]
fn service_open_unknown_managed_or_external_availability_is_unavailable_not_missing() {
    for managed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(&dir.path().join("library.db")).unwrap();
        let mut initial = project("project", 0);
        initial.program_descriptor.components[0].origin = if managed {
            ComponentOrigin::Managed {
                artifact_ref: "unresolved-artifact".into(),
                redistribution_allowed: false,
            }
        } else {
            ComponentOrigin::External {
                url: "https://example.invalid/runtime.zip".into(),
                transport_blake3: "a".repeat(64),
                inventory_blake3: "b".repeat(64),
            }
        };
        save(&mut store, initial.clone(), None);
        let error = project_service::open(&store, "project").unwrap_err();
        assert_eq!(error.code, ErrorCode::ActionUnavailable);
        assert_eq!(
            error.details.get("command"),
            Some(&serde_json::json!("fusion_creator_project_open"))
        );
        assert!(!serde_json::to_string(&error)
            .unwrap()
            .contains("example.invalid"));
    }
}

#[test]
fn service_open_local_external_input_does_not_require_remote_probe() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    let mut initial = project("project", 0);
    initial.program_descriptor.components[0].origin = ComponentOrigin::External {
        url: "https://example.invalid/runtime.zip".into(),
        transport_blake3: "a".repeat(64),
        inventory_blake3: "b".repeat(64),
    };
    save_with_path(&mut store, &initial, dir.path());
    assert!(project_service::open(&store, "project")
        .unwrap()
        .missing_inputs
        .is_empty());
}

#[test]
fn service_open_rejects_bindings_for_components_removed_by_project_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    let initial = project("project", 0);
    save_with_path(&mut store, &initial, dir.path());
    let mut next = project("project", 1);
    next.program_descriptor.components[0].id = "replacement-component".into();
    save(&mut store, next, Some(0));
    assert_eq!(
        project_service::open(&store, "project").unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
}

#[test]
fn service_open_absent_project_is_a_typed_error_and_bad_id_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir.path().join("library.db")).unwrap();
    assert_eq!(
        project_service::open(&store, "absent").unwrap_err().code,
        ErrorCode::SnapshotNotFound
    );
    assert_eq!(
        project_service::open(&store, "").unwrap_err().code,
        ErrorCode::InvalidManifest
    );
}

#[test]
fn service_sql_errors_are_generic_and_do_not_expose_trigger_messages() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_store(&dir.path().join("library.db")).unwrap();
    store.conn.execute_batch("CREATE TRIGGER service_fixture_failure BEFORE INSERT ON creator_program_projects BEGIN SELECT RAISE(ABORT, 'synthetic-private-path-and-credential'); END;").unwrap();
    let error = project_service::save(
        &mut store,
        CreatorProjectSaveRequest {
            project: project("project", 0),
            expected_revision: None,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert!(!serde_json::to_string(&error)
        .unwrap()
        .contains("synthetic-private"));
    assert!(project_storage::open_project(&store, "project")
        .unwrap()
        .is_none());
}
