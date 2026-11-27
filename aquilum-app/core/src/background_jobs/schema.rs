use super::{Error, Result};
use rusqlite::Transaction;

const APPLICATION_ID: i64 = 0x414b4a31; // AKJ1
const SCHEMA_VERSION: i64 = 3;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<()> {
    let application: i64 = tx.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match (application, version) {
        (0, 0) => {
            let empty: bool = tx.query_row(
                "SELECT NOT EXISTS (SELECT 1 FROM sqlite_schema)",
                [],
                |row| row.get(0),
            )?;
            if !empty {
                return Err(Error::Schema);
            }
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        }
        (APPLICATION_ID, 1 | 2 | SCHEMA_VERSION) => {}
        _ => return Err(Error::Schema),
    }
    if version < 2 { tx.execute_batch(include_str!("schema_v2.sql"))?; }
    if version < SCHEMA_VERSION {
        tx.execute_batch(include_str!("schema_v3.sql"))?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}
