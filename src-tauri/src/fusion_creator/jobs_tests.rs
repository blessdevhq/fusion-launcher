use crate::storage::RepositoryStore;

#[test]
fn jobs_schema_is_added_after_v6_and_keeps_legacy_data() {
    let root = tempfile::tempdir().unwrap();
    let store = RepositoryStore::open(&root.path().join("library.db")).unwrap();
    super::jobs_schema::migrate(&store.conn).unwrap();
    let version: i64 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 7);
    for name in [
        "creator_jobs",
        "creator_checkpoint_results",
        "creator_releases",
        "creator_reports",
        "creator_project_metadata",
    ] {
        let count: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing {name}");
    }
}

#[test]
fn migration_preserves_rows_on_database_copy_and_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("copy.db");
    if let Ok(source) = std::env::var("C2_MIGRATION_COPY") {
        std::fs::copy(source, &path).unwrap();
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let tables: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let counts: Vec<i64> = tables
        .iter()
        .map(|table| {
            conn.query_row(
                &format!("SELECT count(*) FROM \"{}\"", table.replace('"', "\"\"")),
                [],
                |row| row.get(0),
            )
            .unwrap()
        })
        .collect();
    drop(conn);
    for _ in 0..2 {
        let store = RepositoryStore::open(&path).unwrap();
        super::jobs_schema::migrate(&store.conn).unwrap();
        for (table, count) in tables.iter().zip(&counts) {
            let actual: i64 = store
                .conn
                .query_row(
                    &format!("SELECT count(*) FROM \"{}\"", table.replace('"', "\"\"")),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(actual, *count, "migration changed legacy row count");
        }
        let integrity: String = store
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let fk: i64 = store
            .conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(fk, 0);
    }
}

#[test]
fn v7_failure_rolls_back_schema_and_does_not_rewrite_v6() {
    let root = tempfile::tempdir().unwrap();
    let store = RepositoryStore::open(&root.path().join("library.db")).unwrap();
    store
        .conn
        .execute_batch(
            "CREATE TABLE creator_jobs(sentinel TEXT); INSERT INTO creator_jobs VALUES('keep');",
        )
        .unwrap();
    assert!(super::jobs_schema::migrate(&store.conn).is_err());
    let version: i64 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 6);
    let created: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='creator_project_metadata'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(created, 0);
    let sentinel: String = store
        .conn
        .query_row("SELECT sentinel FROM creator_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sentinel, "keep");
}
