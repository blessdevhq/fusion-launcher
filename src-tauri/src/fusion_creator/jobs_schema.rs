//! Additive v7 migration. The v6 project/bindings representation stays authoritative.
use rusqlite::{Connection, TransactionBehavior};

pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 7 {
        tx.execute_batch(include_str!("jobs_v7.sql"))?;
    }
    tx.commit()
}
