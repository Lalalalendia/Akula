use super::*;

impl SqliteSyncJournal {
    /// The caller provides a path under app-data, NOT the user vault and NOT
    /// the editor's existing document store.
    pub fn open(path: &Path) -> JournalResult<Self> {
        Self::prepare(Connection::open(path)?)
    }

    /// Never create or migrate a Sync database merely because the user
    /// opened Settings. An older or unknown schema is unavailable, not
    /// evidence that synchronization or recovery is healthy.
    pub(crate) fn open_existing_read_only(path: &Path) -> JournalResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 250;",
        )?;
        let version: i64 = connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        )?;
        if version != SCHEMA_VERSION {
            return Err(JournalError::UnsupportedSchema(version));
        }
        Ok(Self { connection })
    }

    /// Open ONLY an existing current-schema journal for a human-confirmed
    /// non-destructive review event. Never create or migrate on a UI click.
    /// The caller has already validated the host-controlled file location;
    /// cross-process path replacement is not an OS-level security boundary.
    pub(crate) fn open_existing_review_writer(path: &Path) -> JournalResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch(
            "PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        )?;
        if version != SCHEMA_VERSION {
            return Err(JournalError::UnsupportedSchema(version));
        }
        Ok(Self { connection })
    }

    #[cfg(test)]
    pub(super) fn memory() -> JournalResult<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    pub(super) fn prepare(connection: Connection) -> JournalResult<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_operations (
                    operation_id TEXT PRIMARY KEY NOT NULL,
                    vault_id TEXT NOT NULL,
                    document_id TEXT NOT NULL,
                    intent BLOB NOT NULL,
                    digest TEXT NOT NULL,
                    phase INTEGER NOT NULL CHECK(phase BETWEEN 0 AND 4),
                    staged_hash TEXT,
                    readback_hash TEXT,
                    provider_revision TEXT
                 );
                 CREATE UNIQUE INDEX one_inflight_per_document
                    ON sync_operations(vault_id, document_id) WHERE phase < 4;
                 CREATE TABLE sync_baselines (
                    vault_id TEXT NOT NULL,
                    document_id TEXT NOT NULL,
                    revision_id TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    operation_id TEXT NOT NULL,
                    PRIMARY KEY(vault_id, document_id)
                 );
                 CREATE TABLE sync_local_publication_attempts (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_operations(operation_id),
                     vault_id TEXT NOT NULL,
                     path_collision_key TEXT NOT NULL,
                     relative_path TEXT NOT NULL,
                     expected_hash TEXT NOT NULL,
                     intent_digest TEXT NOT NULL,
                     UNIQUE (vault_id, path_collision_key)
                 );
                 CREATE TABLE sync_local_recovery_evidence (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL
                 );
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            1 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_publication_attempts (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_operations(operation_id),
                     vault_id TEXT NOT NULL,
                     path_collision_key TEXT NOT NULL,
                     relative_path TEXT NOT NULL,
                     expected_hash TEXT NOT NULL,
                     intent_digest TEXT NOT NULL,
                     UNIQUE (vault_id, path_collision_key)
                 );
                 CREATE TABLE sync_local_recovery_evidence (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL
                 );
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            2 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_recovery_evidence (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL
                 );
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            3 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            4 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            5 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            6 | 7 | SCHEMA_VERSION => {},
            other => return Err(JournalError::UnsupportedSchema(other)),
        }
        // Older migrations still reach v6. Upgrade atomically on the next
        // writer-open; read-only Settings never creates or migrates tables.
        if version < 7 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_existing_replace_attempts (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_operations(operation_id),
                     vault_id TEXT NOT NULL,
                     document_id TEXT NOT NULL,
                     path_collision_key TEXT NOT NULL,
                     baseline_revision TEXT NOT NULL,
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL,
                     UNIQUE(vault_id,path_collision_key,baseline_revision)
                 );
                 PRAGMA user_version = 7;
                 COMMIT;",
            )?;
        }
        // V8 is an additive, write-once *forensic candidate*, not an S3
        // permission or cross-filesystem atomic transaction. A read-only
        // connection never creates or migrates this table.
        if version < 8 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_existing_replace_receipt_candidates (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_existing_replace_attempts(operation_id),
                     candidate_json BLOB NOT NULL,
                     candidate_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 8;
                 COMMIT;",
            )?;
        }
        Ok(Self { connection })
    }

}
