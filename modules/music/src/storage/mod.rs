//! Account-scoped SQLite history for confirmed normal-speed audio intervals.
//! The host owns session IDs, clock mapping, fencing and exclusive recovery.

mod sessions;

use rusqlite::{Connection, TransactionBehavior, named_params};
use std::path::Path;
use std::time::Duration;

const APPLICATION_ID: i64 = 1095454037;
const VERSION: i64 = 1;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    ForeignDatabase,
    InvalidInput,
    Conflict,
    UnknownSession,
    ActiveSession,
}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredOutcome {
    NaturalEnd,
    Skipped,
    Stopped,
    Failed,
    Interrupted,
}

impl StoredOutcome {
    fn sql(self) -> &'static str {
        match self {
            Self::NaturalEnd => "natural_end",
            Self::Skipped => "skipped",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Totals {
    pub wall_ms: i64,
    pub track_audio_ms: i64,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Use a dedicated, local database path selected by the trusted host.
    /// No implicit interruption of other writers' sessions on open.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(2))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let app: i64 = tx.pragma_query_value(None, "application_id", |r| r.get(0))?;
            let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
            match (app, version) {
                (0, 0) => {
                    let objects: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM sqlite_schema",
                        [],
                        |r| r.get(0),
                    )?;
                    if objects != 0 {
                        return Err(StoreError::ForeignDatabase);
                    }
                    tx.execute_batch(include_str!("schema.sql"))?;
                }
                (APPLICATION_ID, VERSION) => {}
                _ => return Err(StoreError::ForeignDatabase),
            }
            tx.commit()?;
        }
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(StoreError::ForeignDatabase);
        }
        conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { conn })
    }

    /// [lo, hi), UTC milliseconds; crossfade contributes only once to wall_ms.
    pub fn totals(&self, account: &str, lo: i64, hi: i64) -> Result<Totals, StoreError> {
        valid_text(account, 128)?;
        if lo < 0 || hi <= lo {
            return Err(StoreError::InvalidInput);
        }
        Ok(self.conn.query_row(
            include_str!("totals.sql"),
            named_params! { ":account": account, ":lo": lo, ":hi": hi },
            |r| Ok(Totals { wall_ms: r.get(0)?, track_audio_ms: r.get(1)? }),
        )?)
    }
}

fn valid_text(value: &str, max_bytes: usize) -> Result<(), StoreError> {
    if value.trim().is_empty() || value.len() > max_bytes || value.contains('\0') {
        return Err(StoreError::InvalidInput);
    }
    Ok(())
}
