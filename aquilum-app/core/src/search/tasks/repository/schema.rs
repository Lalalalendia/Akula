//! SQLite schema of the indexed Markdown tasks table.
use rusqlite::Connection;

use crate::search::error::SearchError;

pub fn open_schema(connection: &Connection) -> Result<(), SearchError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS note_tasks (
           path TEXT NOT NULL,
           ordinal INTEGER NOT NULL,
           line INTEGER NOT NULL,
           done INTEGER NOT NULL,
           text TEXT NOT NULL,
           PRIMARY KEY(path, ordinal)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS note_tasks_open ON note_tasks(done);",
    )?;
    Ok(())
}
