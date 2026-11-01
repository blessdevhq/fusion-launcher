use super::tests::project;
use super::*;

#[test]
fn typed_open_list_private_cas_and_rollback_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let mut store = RepositoryStore::open(&path).unwrap();
    let parsed =
        CreatorProgramProject::read_json(std::str::from_utf8(&project(0)).unwrap()).unwrap();
    let bindings = CreatorPrivateBindings {
        project_id: "project-1".into(),
        component_paths: std::collections::BTreeMap::from([(
            parsed.program_descriptor.components[0].id.clone(),
            "X:/private-fixture-sentinel".into(),
        )]),
        signing_key_ref: Some("synthetic-key-ref".into()),
    };
    save_project_with_bindings(
        &mut store,
        &project(0),
        None,
        Some(&PrivateBindingsSave {
            revision: 0,
            expected_revision: None,
            bindings: &bindings,
        }),
    )
    .unwrap();
    drop(store);
    let mut store = RepositoryStore::open(&path).unwrap();
    let saved = open_project(&store, "project-1").unwrap().unwrap();
    assert_eq!(saved.revision, 0);
    assert!(!serde_json::to_string(&saved)
        .unwrap()
        .contains("private-fixture-sentinel"));
    assert_eq!(list_projects(&store, 32, 0).unwrap().len(), 1);
    assert!(list_projects(&store, 32, 1).unwrap().is_empty());
    assert!(list_projects(&store, 0, 0).is_err());
    assert!(list_projects(&store, 33, 0).is_err());
    assert!(open_project(&store, "missing").unwrap().is_none());
    assert!(open_private_bindings(&store, "missing").unwrap().is_none());
    let private = open_private_bindings(&store, "project-1").unwrap().unwrap();
    assert_eq!(private.revision, 0);
    assert_eq!(private.bindings.component_paths, bindings.component_paths);
    save_project_bytes(&mut store, &project(1), Some(0)).unwrap();
    assert_eq!(
        open_private_bindings(&store, "project-1")
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    let stale = save_project_with_bindings(
        &mut store,
        &project(2),
        Some(1),
        Some(&PrivateBindingsSave {
            revision: 2,
            expected_revision: Some(1),
            bindings: &bindings,
        }),
    )
    .unwrap_err();
    assert_eq!(stale.code, ErrorCode::SnapshotConflict);
    assert_eq!(
        open_project(&store, "project-1").unwrap().unwrap().revision,
        1
    );
    store.conn.execute_batch("CREATE TRIGGER typed_private_failure BEFORE UPDATE ON creator_private_bindings BEGIN SELECT RAISE(ABORT, 'X:/private-fixture-sentinel'); END;").unwrap();
    let error = save_project_with_bindings(
        &mut store,
        &project(2),
        Some(1),
        Some(&PrivateBindingsSave {
            revision: 1,
            expected_revision: Some(0),
            bindings: &bindings,
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert!(!serde_json::to_string(&error)
        .unwrap()
        .contains("private-fixture-sentinel"));
    drop(store);
    let store = RepositoryStore::open(&path).unwrap();
    assert_eq!(
        open_project(&store, "project-1").unwrap().unwrap().revision,
        1
    );
    assert_eq!(
        open_private_bindings(&store, "project-1")
            .unwrap()
            .unwrap()
            .revision,
        0
    );
}

#[test]
fn typed_reads_reject_digest_consistent_row_metadata_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
    save_project_bytes(&mut store, &project(0), None).unwrap();
    store
        .conn
        .execute("UPDATE creator_program_projects SET revision=1", [])
        .unwrap();
    assert_eq!(
        open_project(&store, "project-1").unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
    store
        .conn
        .execute("UPDATE creator_program_projects SET revision=0", [])
        .unwrap();
    let mut other: serde_json::Value = serde_json::from_slice(&project(0)).unwrap();
    other["project_id"] = "other-project".into();
    for bytes in [serde_json::to_vec(&other).unwrap(), b"{}".to_vec()] {
        let hash = blake3::hash(&bytes).to_hex().to_string();
        store
            .conn
            .execute(
                "UPDATE creator_program_projects SET document=?1,document_blake3=?2",
                params![bytes, hash],
            )
            .unwrap();
        assert_eq!(
            open_project(&store, "project-1").unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        assert!(list_projects(&store, 32, 0).is_err());
    }
}

#[test]
fn typed_private_inputs_are_bound_and_portable_reads_do_not_join_private_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
    let saved = save_project_bytes(&mut store, &project(0), None).unwrap();
    let component = saved.program_descriptor.components[0].id.clone();
    let mut bindings = CreatorPrivateBindings {
        project_id: "other-project".into(),
        component_paths: std::collections::BTreeMap::new(),
        signing_key_ref: None,
    };
    assert!(save_project_with_bindings(
        &mut store,
        &project(1),
        Some(0),
        Some(&PrivateBindingsSave {
            revision: 0,
            expected_revision: None,
            bindings: &bindings,
        })
    )
    .is_err());
    bindings.project_id = "project-1".into();
    bindings
        .component_paths
        .insert("missing-component".into(), "X:/private-fixture".into());
    assert!(save_project_with_bindings(
        &mut store,
        &project(1),
        Some(0),
        Some(&PrivateBindingsSave {
            revision: 0,
            expected_revision: None,
            bindings: &bindings,
        })
    )
    .is_err());
    assert_eq!(
        open_project(&store, "project-1").unwrap().unwrap().revision,
        0
    );
    bindings.component_paths.clear();
    bindings
        .component_paths
        .insert(component, "X:/private-fixture".into());
    save_project_with_bindings(
        &mut store,
        &project(1),
        Some(0),
        Some(&PrivateBindingsSave {
            revision: 0,
            expected_revision: None,
            bindings: &bindings,
        }),
    )
    .unwrap();
    bindings.project_id = "wrong-row".into();
    store
        .conn
        .execute(
            "UPDATE creator_private_bindings SET bindings_json=?1",
            [serde_json::to_vec(&bindings).unwrap()],
        )
        .unwrap();
    assert_eq!(
        open_private_bindings(&store, "project-1").unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
    assert_eq!(
        open_project(&store, "project-1").unwrap().unwrap().revision,
        1
    );
    assert_eq!(list_projects(&store, 32, 0).unwrap().len(), 1);
}

#[test]
fn typed_save_validates_the_program_carrier_before_writing_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
    let bytes = project(0);
    save_project_bytes(&mut store, &bytes, None).unwrap();
    assert_eq!(
        load_document(&store, "project-1").unwrap().unwrap().bytes,
        bytes
    );
    let mut invalid: serde_json::Value = serde_json::from_slice(&project(1)).unwrap();
    invalid["components"] = serde_json::json!([]);
    assert!(
        save_project_bytes(&mut store, &serde_json::to_vec(&invalid).unwrap(), Some(0)).is_err()
    );
    assert_eq!(
        load_document(&store, "project-1").unwrap().unwrap().bytes,
        bytes
    );
}
