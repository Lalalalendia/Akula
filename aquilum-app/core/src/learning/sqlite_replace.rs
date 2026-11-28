//! Atomic full replacement of a validated host-owned metadata singleton.
//! Caller-specific encoding, decoding, schema and error types stay local.

use rusqlite::{Connection, TransactionBehavior, params};

pub(super) enum ReplaceError { Storage, Corrupt, Stale, Exhausted }

/// All SQL is fixed, store-owned SQL. Validate the incumbent before comparing
/// revisions, and never turn a stale or corrupt store into a fresh write.
pub(super) fn replace<E, T>(
    connection: &mut Connection, expected: u64, payload: Vec<u8>,
    select_sql: &'static str, update_sql: &'static str,
    validate: impl FnOnce(&[u8]) -> Result<T, E>, error: impl Fn(ReplaceError) -> E,
) -> Result<u64, E> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| error(ReplaceError::Storage))?;
    let (revision, previous): (i64, Vec<u8>) = tx.query_row(
        select_sql, [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| error(ReplaceError::Corrupt))?;
    validate(&previous)?;
    if u64::try_from(revision).map_err(|_| error(ReplaceError::Corrupt))? != expected {
        return Err(error(ReplaceError::Stale));
    }
    let next = revision.checked_add(1).ok_or_else(|| error(ReplaceError::Exhausted))?;
    let changed = tx.execute(update_sql, params![next, payload, revision])
        .map_err(|_| error(ReplaceError::Storage))?;
    if changed != 1 { return Err(error(ReplaceError::Stale)); }
    tx.commit().map_err(|_| error(ReplaceError::Storage))?;
    u64::try_from(next).map_err(|_| error(ReplaceError::Exhausted))
}
