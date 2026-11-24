//! Shared opening and first initialization of host-owned learning metadata databases.
//! Store-specific SQL, payload validation, revision CAS and authority remain with each caller.

use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params};

#[derive(Debug)]
pub(super) enum OpenError<E> {
    Storage,
    UnsupportedSchema,
    Payload(E),
}

/// The SQL arguments are fixed, store-owned statements, never user or plugin input.
/// Keep the v1 initialization ordering and refuse unknown versions instead of migrating.
pub(super) fn open<E>(
    path: &Path, expected_version: i64, create_sql: &'static str, insert_sql: &'static str,
    empty_payload: impl FnOnce() -> Result<Vec<u8>, E>,
) -> Result<Connection, OpenError<E>> {
    let mut connection = Connection::open(path).map_err(|_| OpenError::Storage)?;
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA busy_timeout=3000;",
    ).map_err(|_| OpenError::Storage)?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| OpenError::Storage)?;
    if version == 0 {
        let existing: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [], |row| row.get(0),
        ).map_err(|_| OpenError::Storage)?;
        if existing != 0 {
            return Err(OpenError::UnsupportedSchema);
        }
        let empty = empty_payload().map_err(OpenError::Payload)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OpenError::Storage)?;
        tx.execute_batch(create_sql).map_err(|_| OpenError::Storage)?;
        tx.execute(insert_sql, params![empty]).map_err(|_| OpenError::Storage)?;
        tx.execute_batch("PRAGMA user_version=1;").map_err(|_| OpenError::Storage)?;
        tx.commit().map_err(|_| OpenError::Storage)?;
    } else if version != expected_version {
        return Err(OpenError::UnsupportedSchema);
    }
    Ok(connection)
}

#[cfg(test)]
#[path = "sqlite_store_tests.rs"]
mod tests;
