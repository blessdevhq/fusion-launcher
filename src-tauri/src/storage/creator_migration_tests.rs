//! Real SQLite upgrade fixtures, not production library data.
use super::*;
use tempfile::tempdir;

#[test]
fn creator_v6_upgrade_preserves_library_and_is_idempotent() {
    let root = tempdir().unwrap();
    let path = root.path().join("library.db");
    let store = RepositoryStore::open(&path).unwrap();
    store.conn.execute("INSERT INTO app_config(key,value,updated_at) VALUES('creator-sentinel','keep','fixture')", []).unwrap();
    store.conn.execute_batch(
        "INSERT INTO repositories(id,url,name,version,schema_version,connected_at,raw_json)
         VALUES('legacy-repository','https://example.com/fixture.json','Legacy repository','1',1,'fixture','{}');
         INSERT INTO catalog_games(id,source_id,repository_id,platform,title,downloads_json,required_system_file_ids_json)
         VALUES('legacy-game','legacy-source','legacy-repository','nes','Legacy title','[]','[]');
         INSERT INTO downloads(subject_id,subject_type,status,local_path,updated_at)
         VALUES('legacy-game','game','completed','F:/fixture/legacy-game.nes','fixture');",
    ).unwrap();
    // Recreate a populated pre-C database from the actual initialized baseline.
    store.conn.execute_batch("DROP TABLE IF EXISTS creator_private_bindings; DROP TABLE IF EXISTS creator_program_projects; PRAGMA user_version=5;").unwrap();
    drop(store);

    for _ in 0..2 {
        let store = RepositoryStore::open(&path).unwrap();
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            version, 6,
            "Creator migration must advance the inspected v5 baseline"
        );
        let sentinel: String = store
            .conn
            .query_row(
                "SELECT value FROM app_config WHERE key='creator-sentinel'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sentinel, "keep");
        let preserved: (String, String, String, String, String) = store
            .conn
            .query_row(
                "SELECT r.name,c.title,d.status,d.local_path,r.raw_json
             FROM repositories r JOIN catalog_games c ON c.repository_id=r.id
             JOIN downloads d ON d.subject_id=c.id WHERE c.id='legacy-game'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            preserved,
            (
                "Legacy repository".into(),
                "Legacy title".into(),
                "completed".into(),
                "F:/fixture/legacy-game.nes".into(),
                "{}".into(),
            )
        );
        let foreign_key_errors: i64 = store
            .conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(foreign_key_errors, 0);
        for table in ["creator_program_projects", "creator_private_bindings"] {
            let count: i64 = store
                .conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
    }
}

#[test]
fn creator_v6_second_table_failure_rolls_back_first_table_and_schema_version() {
    let root = tempdir().unwrap();
    let path = root.path().join("library.db");
    let store = RepositoryStore::open(&path).unwrap();
    store
        .conn
        .execute_batch(
            "DROP TABLE creator_private_bindings;
         DROP TABLE creator_program_projects;
         PRAGMA user_version=5;
         CREATE TABLE creator_private_bindings(sentinel TEXT NOT NULL);
         INSERT INTO creator_private_bindings VALUES('preserve-unrelated-data');",
        )
        .unwrap();
    drop(store);
    assert!(RepositoryStore::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
    let first_table_count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='creator_program_projects'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        first_table_count, 0,
        "first CREATE must roll back with the second CREATE"
    );
    let sentinel: String = connection
        .query_row("SELECT sentinel FROM creator_private_bindings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(sentinel, "preserve-unrelated-data");
}
