//! Isolated QA model for a durable, two-path transition reservation.
//! This is NOT Akula's installer-owned host registry, not a File Gate guard,
//! not an S3/HistoryService lease and NOT permission for a runtime move.
//! Only disposable test paths and an independently provisioned fixture DB
//! are ever modified. A same-user attacker can forge this entire database.
use crate::search::paths::identity;
use crate::sync::filegate_lock_probe::PinnedProbeRoot;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const SCHEMA: i64 = 1;
const MAX_RESERVATIONS: usize = 512;

#[derive(Debug)]
enum Denial {
    UntrustedOrInvalid,
    PathReserved,
    Database(rusqlite::Error),
}
impl From<rusqlite::Error> for Denial {
    fn from(e: rusqlite::Error) -> Self { Self::Database(e) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Reservation {
    operation: Uuid,
    workspace: Uuid,
    source: PathBuf,
    destination: PathBuf,
    root_key: String,
}

fn overlap(a: &Path, b: &Path) -> bool {
    let a = identity(a);
    let b = identity(b);
    let a = a.trim_end_matches('/');
    let b = b.trim_end_matches('/');
    a == b || a.starts_with(&(b.to_owned() + "/"))
        || b.starts_with(&(a.to_owned() + "/"))
}

fn existing_rows(conn: &Connection) -> Result<Vec<Reservation>, Denial> {
    let mut stmt = conn.prepare(
        "SELECT operation, workspace, source_path, destination_path, root_key FROM reservations"
    )?;
    let records = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?,
            r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
    })?;
    let mut result = Vec::new();
    for item in records {
        let (operation, workspace, source, destination, root_key) = item?;
        if result.len() == MAX_RESERVATIONS { return Err(Denial::UntrustedOrInvalid); }
        result.push(Reservation {
            operation: Uuid::parse_str(&operation).map_err(|_| Denial::UntrustedOrInvalid)?,
            workspace: Uuid::parse_str(&workspace).map_err(|_| Denial::UntrustedOrInvalid)?,
            source: PathBuf::from(source),
            destination: PathBuf::from(destination),
            root_key,
        });
    }
    Ok(result)
}

fn observed_request(operation: Uuid, workspace: Uuid,
                    source: &Path, destination: &Path) -> Result<Reservation, Denial> {
    if operation.is_nil() || workspace.is_nil() {
        return Err(Denial::UntrustedOrInvalid);
    }
    // The input must have a real, plain source directory. Paths are
    // observations only: no immutable namespace pin is granted.
    let meta = fs::symlink_metadata(source).map_err(|_| Denial::UntrustedOrInvalid)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Denial::UntrustedOrInvalid);
    }
    let source = fs::canonicalize(source).map_err(|_| Denial::UntrustedOrInvalid)?;
    let parent = source.parent().ok_or(Denial::UntrustedOrInvalid)?;
    if !destination.is_absolute()
        || !overlap(parent, &destination.parent().ok_or(Denial::UntrustedOrInvalid)?.to_path_buf())
        || identity(parent) != identity(destination.parent().unwrap())
        || overlap(&source, destination)
        || destination.file_name().is_none()
        || fs::symlink_metadata(destination).is_ok()
    {
        return Err(Denial::UntrustedOrInvalid);
    }
    // The pinned physical ID is just an additional mismatch detector,
    // not cross-process ownership proof, and it may be recycled.
    let (volume, file) = PinnedProbeRoot::open(&source)
        .map_err(|_| Denial::UntrustedOrInvalid)?.identity();
    if file == 0 { return Err(Denial::UntrustedOrInvalid); }
    Ok(Reservation {
        operation, workspace, source,
        destination: destination.to_path_buf(),
        root_key: format!("{volume:016x}:{file:016x}"),
    })
}

struct FixtureLedger {
    connection: Connection,
}
impl FixtureLedger {
    /// Only the test harness provisions its own empty host-like SQLite.
    fn provision_fixture(path: &Path) -> Result<Self, Denial> {
        if path.exists() { return Err(Denial::UntrustedOrInvalid); }
        let connection = Connection::open_with_flags(path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE reservations (
                operation TEXT NOT NULL PRIMARY KEY,
                workspace TEXT NOT NULL,
                source_path TEXT NOT NULL,
                destination_path TEXT NOT NULL,
                root_key TEXT NOT NULL
             );
             PRAGMA user_version=1;"
        )?;
        Ok(Self { connection })
    }

    fn open_existing(path: &Path) -> Result<Self, Denial> {
        let meta = fs::symlink_metadata(path).map_err(|_| Denial::UntrustedOrInvalid)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(Denial::UntrustedOrInvalid);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(Denial::UntrustedOrInvalid);
            }
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(Duration::from_millis(600))?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != SCHEMA || connection.query_row(
            "PRAGMA quick_check", [], |r| r.get::<_, String>(0)
        )? != "ok" { return Err(Denial::UntrustedOrInvalid); }
        existing_rows(&connection)?; // a corrupted row must HOLD, not disappear
        Ok(Self { connection })
    }

    /// BEGIN IMMEDIATE serializes only COOPERATING SQLite connections.
    /// Reserving both names across process restarts is necessary but not
    /// sufficient: ordinary Core/History/S3 never consult this fixture.
    fn reserve(&mut self, request: &Reservation) -> Result<(), Denial> {
        if request.operation.is_nil() || request.workspace.is_nil()
            || request.source == request.destination {
            return Err(Denial::UntrustedOrInvalid);
        }
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for row in existing_rows(&tx)? {
            if row.operation == request.operation || row.workspace == request.workspace
                || row.root_key == request.root_key
                || overlap(&row.source, &request.source)
                || overlap(&row.source, &request.destination)
                || overlap(&row.destination, &request.source)
                || overlap(&row.destination, &request.destination)
            { return Err(Denial::PathReserved); }
        }
        tx.execute(
            "INSERT INTO reservations (operation, workspace, source_path, destination_path, root_key)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![request.operation.to_string(), request.workspace.to_string(),
                request.source.to_string_lossy(), request.destination.to_string_lossy(),
                request.root_key],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn inspect(&self) -> Result<Vec<Reservation>, Denial> {
        existing_rows(&self.connection)
    }

    /// Read-only negative evidence. A false result is NOT write permission.
    fn would_hold(&self, candidate: &Path) -> Result<bool, Denial> {
        Ok(self.inspect()?.iter().any(|r|
            overlap(candidate, &r.source) || overlap(candidate, &r.destination)))
    }
}

#[cfg(test)]
mod tests;
