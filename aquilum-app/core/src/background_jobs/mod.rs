//! Durable, device-local background work with atomic output checkpoints.

use rusqlite::{params, Connection, ErrorCode, OptionalExtension, Row, TransactionBehavior};
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod speech;
#[cfg(test)]
mod tests;
mod worker;

const APPLICATION_ID: i64 = 0x414b4a31; // AKJ1
const SCHEMA_VERSION: i64 = 1;
const MAX_PAYLOAD: usize = 256 * 1024;
const MAX_SEGMENTS: i64 = 1_000_000;
pub type JobId = i64;

#[derive(Debug)]
pub enum Error {
    Database(rusqlite::Error),
    Schema,
    Invalid(&'static str),
    Conflict,
    Clock,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => write!(f, "{error}"),
            Self::Schema => f.write_str("database is not a supported Akula background queue"),
            Self::Invalid(field) => write!(f, "invalid background job {field}"),
            Self::Conflict => f.write_str("job, lease or segment checkpoint changed"),
            Self::Clock => f.write_str("invalid system clock for background jobs"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Spec {
    pub kind: String,
    pub source: String,
    pub revision: String,
    pub recipe: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Running,
    Paused,
    Cancelled,
    Failed,
    Completed,
}

impl State {
    fn read(row: &Row<'_>, index: usize) -> rusqlite::Result<Self> {
        match row.get_ref(index)?.as_str()? {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "completed" => Ok(Self::Completed),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: JobId,
    pub spec: Spec,
    pub state: State,
    pub committed: i64,
    pub total: Option<i64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment {
    pub ordinal: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct Lease {
    job: JobId,
    generation: i64,
}

impl Lease {
    pub fn job(&self) -> JobId {
        self.job
    }
}

pub struct Queue {
    connection: Connection,
}

const JOB_FIELDS: &str = "id,kind,source,revision,recipe,state,committed,total,error";

fn read_job(row: &Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        spec: Spec {
            kind: row.get(1)?,
            source: row.get(2)?,
            revision: row.get(3)?,
            recipe: row.get(4)?,
        },
        state: State::read(row, 5)?,
        committed: row.get(6)?,
        total: row.get(7)?,
        error: row.get(8)?,
    })
}

fn changed(count: usize) -> Result<()> {
    if count == 1 {
        Ok(())
    } else {
        Err(Error::Conflict)
    }
}

fn now() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Clock)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| Error::Clock)
}

fn valid_text(field: &'static str, value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        Err(Error::Invalid(field))
    } else {
        Ok(())
    }
}

fn enable_wal(connection: &Connection) -> Result<()> {
    let started = Instant::now();
    loop {
        match connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0)) {
            Ok(mode) if mode == "wal" => return Ok(()),
            Ok(_) => return Err(Error::Schema),
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                ) && started.elapsed() < Duration::from_secs(5) =>
            {
                // Opening the database concurrently can bypass the busy
                // handler during SQLite's rollback-journal-to-WAL transition.
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

impl Queue {
    /// Open a dedicated queue database inside the device's local profile.
    pub fn open(path: &Path) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;

        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            (APPLICATION_ID, SCHEMA_VERSION) => {}
            _ => return Err(Error::Schema),
        }
        tx.commit()?;
        // WAL allows UI readers while a worker commits. FULL preserves completed
        // transactions across a process or host crash, subject to filesystem guarantees.
        enable_wal(&connection)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { connection })
    }

    /// The same source revision and recipe always return the same job,
    /// including after completion or cancellation.
    pub fn enqueue(&self, spec: &Spec) -> Result<JobId> {
        valid_text("kind", &spec.kind, 64)?;
        valid_text("source", &spec.source, 16 * 1024)?;
        valid_text("recipe", &spec.recipe, 4096)?;
        if spec.revision.len() != 64
            || !spec
                .revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid("revision"));
        }
        let identity = params![spec.kind, spec.source, spec.revision, spec.recipe];
        self.connection.execute(
            "INSERT INTO jobs (kind,source,revision,recipe) VALUES (?1,?2,?3,?4)
             ON CONFLICT(kind,source,revision,recipe) DO NOTHING",
            identity,
        )?;
        Ok(self.connection.query_row(
            "SELECT id FROM jobs WHERE kind=?1 AND source=?2 AND revision=?3 AND recipe=?4",
            identity,
            |row| row.get(0),
        )?)
    }

    pub fn get(&self, id: JobId) -> Result<Option<Job>> {
        Ok(self
            .connection
            .query_row(
                &format!("SELECT {JOB_FIELDS} FROM jobs WHERE id=?1"),
                [id],
                read_job,
            )
            .optional()?)
    }

    pub fn list(&self, kind: &str, before: Option<JobId>, limit: u32) -> Result<Vec<Job>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {JOB_FIELDS} FROM jobs WHERE kind=?1 AND (?2 IS NULL OR id<?2)
             ORDER BY id DESC LIMIT ?3"
        ))?;
        let rows = statement.query_map(params![kind, before, limit.clamp(1, 100)], read_job)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ordinal-based pagination supports live readers without loading all output.
    pub fn segments(&self, id: JobId, first: i64, limit: u32) -> Result<Vec<Segment>> {
        let mut statement = self.connection.prepare(
            "SELECT ordinal,start_ms,end_ms,payload FROM segments
             WHERE job=?1 AND ordinal>=?2 ORDER BY ordinal LIMIT ?3",
        )?;
        let rows = statement.query_map(params![id, first, limit.clamp(1, 100)], |row| {
            Ok(Segment {
                ordinal: row.get(0)?,
                start_ms: row.get(1)?,
                end_ms: row.get(2)?,
                payload: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn pause(&self, id: JobId) -> Result<()> {
        changed(self.connection.execute(
            "UPDATE jobs SET state='paused',lease_until=NULL WHERE id=?1 AND state IN ('queued','running')",
            [id],
        )?)
    }

    pub fn resume(&self, id: JobId) -> Result<()> {
        changed(self.connection.execute(
            "UPDATE jobs SET state='queued',error=NULL WHERE id=?1 AND state IN ('paused','failed')",
            [id],
        )?)
    }

    pub fn cancel(&self, id: JobId) -> Result<()> {
        changed(self.connection.execute(
            "UPDATE jobs SET state='cancelled',lease_until=NULL,error=NULL
             WHERE id=?1 AND state IN ('queued','running','paused','failed')",
            [id],
        )?)
    }
}
