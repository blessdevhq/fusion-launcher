use super::*;
use tempfile::tempdir;

pub(super) fn project(revision: u64) -> Vec<u8> {
    let descriptor: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/creator/programs/supertuxkart-1.5.json"
    ))
    .unwrap();
    serde_json::to_vec(&serde_json::json!({
        "schema_version": "1.1", "project_id": "project-1", "revision": revision,
        "title": "Storage fixture", "source_id": "source", "game_id": "game",
        "game_version": "1.5", "program_descriptor": descriptor
    }))
    .unwrap()
}

// Test-only trusted fixture envelope; not a public unvalidated JSON entrypoint.
fn fixture_document(bytes: &[u8], revision: u64) -> VerifiedDocument<'_> {
    VerifiedDocument {
        project_id: "project-1",
        revision,
        bytes,
    }
}

#[test]
fn saves_real_frozen_fixture_as_one_authoritative_document() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.db");
    let mut store = RepositoryStore::open(&path).unwrap();
    let bytes = project(0);
    let bindings =
        br#"{"local_path":"C:/private/game","device":"private-device","key_ref":"private-key"}"#;
    let update = PrivateBindingUpdate {
        revision: 0,
        expected_revision: None,
        bytes: bindings,
    };
    store_verified(
        &mut store,
        &fixture_document(&bytes, 0),
        None,
        Some(&update),
    )
    .unwrap();
    drop(store);
    let mut store = RepositoryStore::open(&path).unwrap();
    let saved = load_document(&store, "project-1").unwrap().unwrap();
    assert_eq!(
        saved,
        StoredBytes {
            revision: 0,
            bytes: bytes.clone()
        }
    );
    assert_eq!(
        load_private_bindings(&store, "project-1")
            .unwrap()
            .unwrap()
            .bytes,
        bindings
    );
    assert!(!String::from_utf8(saved.bytes).unwrap().contains("private-"));
    assert!(
        !dir.path().join("project-1.json").exists(),
        "no second mutable representation"
    );
    assert!(load_document(&store, "missing").unwrap().is_none());

    // Another real connection has a stale expected revision after this writer wins.
    let mut stale = RepositoryStore::open(&path).unwrap();
    let next = project(1);
    store_verified(&mut store, &fixture_document(&next, 1), Some(0), None).unwrap();
    let err = store_verified(&mut stale, &fixture_document(&next, 1), Some(0), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::SnapshotConflict);
    let err = store_verified(&mut stale, &fixture_document(&bytes, 0), None, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::SnapshotConflict);
    assert_eq!(
        load_document(&stale, "project-1").unwrap().unwrap().bytes,
        next
    );

    // A failure in the second table must roll back the first table, too.
    store.conn.execute_batch("CREATE TRIGGER reject_private BEFORE UPDATE ON creator_private_bindings BEGIN SELECT RAISE(ABORT, 'fixture disk/write failure'); END;").unwrap();
    let third = project(2);
    let private_next = PrivateBindingUpdate {
        revision: 1,
        expected_revision: Some(0),
        bytes: br#"{"device":"replacement"}"#,
    };
    assert!(store_verified(
        &mut store,
        &fixture_document(&third, 2),
        Some(1),
        Some(&private_next)
    )
    .is_err());
    drop(store);
    let mut store = RepositoryStore::open(&path).unwrap();
    assert_eq!(
        load_document(&store, "project-1").unwrap().unwrap(),
        StoredBytes {
            revision: 1,
            bytes: next
        }
    );
    assert_eq!(
        load_private_bindings(&store, "project-1")
            .unwrap()
            .unwrap()
            .bytes,
        bindings
    );
    store
        .conn
        .execute_batch("DROP TRIGGER reject_private")
        .unwrap();
    let wrong_private = PrivateBindingUpdate {
        revision: 2,
        expected_revision: Some(1),
        bytes: b"{}",
    };
    assert_eq!(
        store_verified(
            &mut store,
            &fixture_document(&third, 2),
            Some(1),
            Some(&wrong_private)
        )
        .unwrap_err()
        .code,
        ErrorCode::SnapshotConflict
    );
    assert_eq!(
        load_document(&store, "project-1")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    store_verified(
        &mut store,
        &fixture_document(&third, 2),
        Some(1),
        Some(&private_next),
    )
    .unwrap();
    assert_eq!(
        load_document(&store, "project-1")
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        load_private_bindings(&store, "project-1")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn revision_bounds_and_corruption_fail_closed() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
    let bytes = project(0);
    assert!(store_verified(
        &mut store,
        &fixture_document(&bytes, 9007199254740992),
        None,
        None
    )
    .is_err());
    assert!(load_document(&store, "project-1").unwrap().is_none());
    let large = vec![b'x'; 2097153];
    assert!(store_verified(&mut store, &fixture_document(&large, 0), None, None).is_err());
    store_verified(&mut store, &fixture_document(&bytes, 0), None, None).unwrap();
    let jump = project(2);
    assert!(store_verified(&mut store, &fixture_document(&jump, 2), Some(0), None).is_err());
    store
        .conn
        .execute("UPDATE creator_program_projects SET document=x'7b7d'", [])
        .unwrap();
    assert_eq!(
        load_document(&store, "project-1").unwrap_err().code,
        ErrorCode::IntegrityViolation
    );
}

#[test]
fn typed_production_facade_rejects_missing_expected_row_without_writes() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
    assert_eq!(
        save_project_bytes(&mut store, &project(1), Some(0))
            .unwrap_err()
            .code,
        ErrorCode::SnapshotConflict
    );
    let count: i64 = store
        .conn
        .query_row("SELECT count(*) FROM creator_program_projects", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn highest_revision_is_readable_but_cannot_be_advanced() {
    let dir = tempdir().unwrap();
    let mut store = RepositoryStore::open(&dir.path().join("library.db")).unwrap();
    let bytes = project(MAX_REVISION);
    store_verified(
        &mut store,
        &fixture_document(&bytes, MAX_REVISION),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        load_document(&store, "project-1").unwrap().unwrap(),
        StoredBytes {
            revision: MAX_REVISION,
            bytes: bytes.clone(),
        }
    );
    let error = store_verified(
        &mut store,
        &fixture_document(&bytes, MAX_REVISION),
        Some(MAX_REVISION),
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SnapshotConflict);
    assert!(store_verified(
        &mut store,
        &fixture_document(&project(MAX_REVISION + 1), MAX_REVISION + 1),
        Some(MAX_REVISION),
        None,
    )
    .is_err());
    assert_eq!(
        load_document(&store, "project-1").unwrap().unwrap().bytes,
        bytes
    );
}

#[test]
fn omitted_private_update_preserves_its_revision_and_sql_errors_are_redacted() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.db");
    let mut store = RepositoryStore::open(&path).unwrap();
    let bytes = project(0);
    let private = br#"{"local_path":"C:/private/sensitive-fixture"}"#;
    store_verified(
        &mut store,
        &fixture_document(&bytes, 0),
        None,
        Some(&PrivateBindingUpdate {
            revision: 0,
            expected_revision: None,
            bytes: private,
        }),
    )
    .unwrap();
    let next = project(1);
    store_verified(&mut store, &fixture_document(&next, 1), Some(0), None).unwrap();
    assert_eq!(
        load_private_bindings(&store, "project-1").unwrap().unwrap(),
        StoredBytes {
            revision: 0,
            bytes: private.to_vec(),
        }
    );
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER reject_private_secret BEFORE UPDATE ON creator_private_bindings
         BEGIN SELECT RAISE(ABORT, 'C:/private/sensitive-fixture'); END;",
        )
        .unwrap();
    let third = project(2);
    let error = store_verified(
        &mut store,
        &fixture_document(&third, 2),
        Some(1),
        Some(&PrivateBindingUpdate {
            revision: 1,
            expected_revision: Some(0),
            bytes: b"{}",
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    let serialized_error = serde_json::to_string(&error).unwrap();
    assert!(!serialized_error.contains("sensitive-fixture"));
    assert!(!serialized_error.contains("C:/private"));
    drop(store);
    let store = RepositoryStore::open(&path).unwrap();
    assert_eq!(
        load_document(&store, "project-1").unwrap().unwrap().bytes,
        next
    );
    assert_eq!(
        load_private_bindings(&store, "project-1").unwrap().unwrap(),
        StoredBytes {
            revision: 0,
            bytes: private.to_vec(),
        }
    );
}
