//! Persistence for opt-in Notion one-way schedule policy.
//! Sidecar appdata SQLite, created only after an explicit user action.
//! Never stores credentials, Notion document contents, or user Vault paths.
//! Runtime workers remain disabled until a separate integration gate.
use super::{Hold, RootPolicy, SyncQueue, MAX_ROOTS, MIN_INTERVAL_MS};
use rusqlite::{params, Connection, OpenFlags};
use std::io::ErrorKind;
use std::path::Path;

const VERSION: i64 = 1;
const DDL: &str = "
CREATE TABLE IF NOT EXISTS notion_refresh_roots (
    root_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    interval_ms INTEGER NOT NULL CHECK (interval_ms >= 900000),
    next_due_ms INTEGER NOT NULL CHECK (next_due_ms >= 0),
    last_verified_ms INTEGER CHECK (last_verified_ms >= 0),
    consecutive_failures INTEGER NOT NULL CHECK (consecutive_failures >= 0),
    hold TEXT NOT NULL CHECK (hold IN ('', 'auth', 'permission', 'review', 'tempfail'))
);
";

pub struct PolicyDb {
    connection: Connection,
}

impl PolicyDb {
    /// Only the user's verified application data directory should be used.
    /// This method intentionally creates the sidecar and must not be called
    /// from startup, read-only view rendering or default settings discovery.
    pub fn open_for_opt_in(path: &Path) -> Result<Self, String> {
        validate_existing_regular_file(path)?;
        let connection = Connection::open_with_flags(
            path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        ).map_err(|_| "cannot open Notion schedule sidecar")?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 2000;",
        ).map_err(|_| "cannot set durable Notion schedule transaction policy")?;
        let version = schema_version(&connection)?;
        if version == 0 {
            connection.execute_batch(&format!(
                "BEGIN IMMEDIATE; {DDL} PRAGMA user_version = {VERSION}; COMMIT;"
            )).map_err(|_| "cannot create Notion schedule schema")?;
        } else if version != VERSION {
            return Err("unsupported Notion schedule schema; refusing downgrade".into());
        }
        Ok(Self { connection })
    }

    /// Missing sidecar means 'no scheduled roots' and causes NO file write.
    /// Read-only access cannot create or update a schedule on disk.
    pub fn open_existing(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("cannot inspect Notion schedule file".into()),
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() =>
                return Err("Notion schedule path is not a regular file".into()),
            Ok(_) => {}
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "cannot open existing Notion schedule read-only")?;
        connection.execute_batch("PRAGMA query_only = ON;")
            .map_err(|_| "cannot protect Notion schedule reader")?;
        if schema_version(&connection)? != VERSION {
            return Err("unsupported Notion schedule schema; refusing to read".into());
        }
        Ok(Some(Self { connection }))
    }

    /// Save all registered root state in ONE transaction. There is intentionally
    /// no DELETE: a partial/empty application state cannot silently erase
    /// user opt-ins or Retry-After holds belonging to a prior process.
    pub fn save(&mut self, queue: &SyncQueue) -> Result<(), String> {
        if queue.roots.len() > MAX_ROOTS || queue.interval_ms < MIN_INTERVAL_MS {
            return Err("invalid Notion schedule queue".into());
        }
        let interval = encode_num(queue.interval_ms)?;
        // A stale in-memory scheduler must not leave existing enabled roots
        // silently running with policy that the caller did not load.
        {
            let mut stmt = self.connection.prepare(
                "SELECT root_id FROM notion_refresh_roots ORDER BY root_id LIMIT 33",
            ).map_err(|_| "cannot verify Notion schedule ownership")?;
            let stored = stmt.query_map([], |row| row.get::<_, String>(0))
                .map_err(|_| "cannot enumerate existing Notion schedules")?;
            for row in stored {
                let id = row.map_err(|_| "invalid stored Notion schedule key")?;
                if !queue.roots.contains_key(&id) {
                    return Err("Notion schedule has roots missing from caller state".into());
                }
            }
        }
        let tx = self.connection.transaction()
            .map_err(|_| "cannot start Notion schedule transaction")?;
        for (id, root) in &queue.roots {
            if id != &root.root {
                return Err("Notion scheduled UUID mismatch".into());
            }
            let next_due = encode_num(root.next_due_ms)?;
            let verified = root.last_verified_ms.map(encode_num).transpose()?;
            let failures = i64::from(root.consecutive_failures);
            tx.execute(
                "INSERT INTO notion_refresh_roots
                    (root_id, enabled, interval_ms, next_due_ms, last_verified_ms,
                     consecutive_failures, hold)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(root_id) DO UPDATE SET
                    enabled=excluded.enabled, interval_ms=excluded.interval_ms,
                    next_due_ms=excluded.next_due_ms,
                    last_verified_ms=excluded.last_verified_ms,
                    consecutive_failures=excluded.consecutive_failures,
                    hold=excluded.hold",
                params![id, if root.enabled { 1_i64 } else { 0_i64 }, interval, next_due, verified,
                    failures, hold_to_str(root.hold)],
            ).map_err(|_| "cannot stage Notion schedule policy")?;
        }
        tx.commit().map_err(|_| "cannot commit Notion schedule policy")?;
        Ok(())
    }

    /// Restored workers are ALWAYS idle; no in-flight task is treated as
    /// completed after a crash. The next due attempt still goes through the
    /// same read-only fetch and atomic snapshot publication gates.
    pub fn load(&self, interval_ms: u64) -> Result<SyncQueue, String> {
        let mut queue = SyncQueue::new(interval_ms)?;
        let mut statement = self.connection.prepare(
            "SELECT root_id, enabled, interval_ms, next_due_ms, last_verified_ms,
                    consecutive_failures, hold
             FROM notion_refresh_roots ORDER BY root_id LIMIT 33",
        ).map_err(|_| "cannot prepare Notion schedule reader")?;
        let rows = statement.query_map([], |row| Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, String>(6)?,
        ))).map_err(|_| "cannot read Notion schedule root list")?;
        for row in rows {
            let (root, enabled, interval, due, verified, failures, hold) =
                row.map_err(|_| "invalid Notion schedule row")?;
            if queue.roots.len() >= MAX_ROOTS ||
                super::canonical_root(&root).ok().as_deref() != Some(root.as_str())
            {
                return Err("invalid or excessive Notion root in stored schedule".into());
            }
            if enabled != 0 && enabled != 1 { return Err("invalid stored Notion opt-in".into()); }
            if u64::try_from(interval).ok() != Some(interval_ms) {
                return Err("Notion schedule interval mismatch; explicit update required".into());
            }
            let state = RootPolicy {
                root: root.clone(), enabled: enabled == 1,
                next_due_ms: decode_num(due)?,
                last_verified_ms: verified.map(decode_num).transpose()?,
                consecutive_failures: u32::try_from(failures)
                    .map_err(|_| "invalid Notion retry count")?,
                hold: str_to_hold(&hold)?,
            };
            queue.roots.insert(root, state);
        }
        Ok(queue)
    }
}

fn validate_existing_regular_file(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() =>
            Err("Notion schedule file must be a regular file, not a symlink".into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err("cannot inspect Notion schedule file".into()),
    }
}

fn schema_version(conn: &Connection) -> Result<i64, String> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| "cannot inspect Notion schedule schema".into())
}

fn encode_num(v: u64) -> Result<i64, String> {
    i64::try_from(v).map_err(|_| "Notion schedule timestamp overflow".into())
}
fn decode_num(v: i64) -> Result<u64, String> {
    u64::try_from(v).map_err(|_| "negative Notion schedule timestamp".into())
}
fn hold_to_str(hold: Option<Hold>) -> &'static str {
    match hold {
        None => "",
        Some(Hold::Authorization) => "auth",
        Some(Hold::Permission) => "permission",
        Some(Hold::NeedsReview) => "review",
        Some(Hold::TemporaryFailures) => "tempfail",
    }
}
fn str_to_hold(value: &str) -> Result<Option<Hold>, String> {
    match value {
        "" => Ok(None),
        "auth" => Ok(Some(Hold::Authorization)),
        "permission" => Ok(Some(Hold::Permission)),
        "review" => Ok(Some(Hold::NeedsReview)),
        "tempfail" => Ok(Some(Hold::TemporaryFailures)),
        _ => Err("unrecognized Notion schedule hold status".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Outcome, MIN_INTERVAL_MS};
    const ROOT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

    #[test]
    fn no_opt_in_means_no_schedule_file_and_no_network() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notion-schedule.sqlite3");
        assert!(PolicyDb::open_existing(&path).unwrap().is_none());
        assert!(!path.exists());
        let mut queue = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        queue.enroll(ROOT, 10).unwrap();
        assert_eq!(queue.claim_due(10), None);
        assert!(!path.exists());
    }

    #[test]
    fn disabled_or_paused_root_remains_safe_after_database_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notion-schedule.sqlite3");
        let mut policy = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        policy.enroll(ROOT, 1).unwrap();
        {
            let mut db = PolicyDb::open_for_opt_in(&path).unwrap();
            db.save(&policy).unwrap();
        }
        {
            let db = PolicyDb::open_existing(&path).unwrap().unwrap();
            let restored = db.load(MIN_INTERVAL_MS).unwrap();
            assert!(!restored.status(ROOT).unwrap().enabled);
            assert_eq!(restored.status(ROOT).unwrap().last_verified_ms, None);
        }
        policy.set_enabled(ROOT, true, 2).unwrap();
        policy.claim_due(2).unwrap();
        policy.finish(ROOT, Outcome::Throttled { retry_after_seconds: Some(3_600) }, 3).unwrap();
        let before = policy.status(ROOT).unwrap().next_due_ms;
        {
            let mut db = PolicyDb::open_for_opt_in(&path).unwrap();
            db.save(&policy).unwrap();
        }
        let db = PolicyDb::open_existing(&path).unwrap().unwrap();
        let mut restored = db.load(MIN_INTERVAL_MS).unwrap();
        assert_eq!(restored.status(ROOT).unwrap().next_due_ms, before);
        assert_eq!(restored.claim_due(before - 1), None);
        assert_eq!(restored.claim_due(before).as_deref(), Some(ROOT));
    }

    #[test]
    fn invalid_schema_is_not_overwritten_or_downgraded() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notion-schedule.sqlite3");
        {
            let db = PolicyDb::open_for_opt_in(&path).unwrap();
            db.connection.execute_batch("PRAGMA user_version=99;").unwrap();
        }
        assert!(PolicyDb::open_for_opt_in(&path).is_err());
        assert!(PolicyDb::open_existing(&path).is_err());
        assert_eq!(Connection::open(&path).unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap(), 99);
    }

    #[test]
    fn stale_in_memory_state_cannot_leave_unknown_enabled_roots_running() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notion-schedule.sqlite3");
        let mut q = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        q.enroll(ROOT, 0).unwrap();
        q.set_enabled(ROOT, true, 0).unwrap();
        let mut db = PolicyDb::open_for_opt_in(&path).unwrap();
        db.save(&q).unwrap();
        let stale = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        assert!(db.save(&stale).is_err());
        let restored = db.load(MIN_INTERVAL_MS).unwrap();
        assert!(restored.status(ROOT).unwrap().enabled);
    }

    #[test]
    fn state_store_has_no_secrets_or_document_content_columns() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notion-schedule.sqlite3");
        let db = PolicyDb::open_for_opt_in(&path).unwrap();
        let schema: String = db.connection.query_row(
            "SELECT sql FROM sqlite_master WHERE name='notion_refresh_roots'",
            [], |row| row.get(0),
        ).unwrap();
        assert!(!schema.contains("token"));
        assert!(!schema.contains("password"));
        assert!(!schema.contains("raw_json"));
        assert!(!schema.contains("content"));
        assert!(!schema.contains("vault"));
    }
}
