//! Opt-in laboratory for a *rebuildable* semantic index.
//!
//! This is not a source of truth, a claim assessor, or an authorization layer.
//! Callers must resolve the current source revision and allowed source IDs
//! using the trusted host BEFORE calling `search`. There is no MCP/UI route.
//! The bounded linear scan is intentional for small acceptance corpora; it is
//! not an approximate-nearest-neighbor implementation.
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;
use std::fmt;

const MAX_DIMENSION: usize = 4096;
const MAX_CHUNKS_PER_SOURCE: usize = 1024;
const MAX_SCANNED_CHUNKS: usize = 8192;
const MAX_RESULTS: usize = 50;

#[derive(Debug)]
pub enum ProbeError {
    Invalid(&'static str),
    CorruptVector,
    ScanBudgetExceeded,
    Database(rusqlite::Error),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(why) => f.write_str(why),
            Self::CorruptVector => f.write_str("Stored vector is corrupt or has the wrong dimension"),
            Self::ScanBudgetExceeded => f.write_str("Semantic probe scan budget exceeded"),
            Self::Database(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ProbeError {}

impl From<rusqlite::Error> for ProbeError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

pub struct Chunk<'a> {
    /// Stable locator within the immutable source revision, not an absolute path.
    pub locator: &'a str,
    /// BLAKE3 of the extracted fragment. Extractor/chunker version belongs in
    /// the model key or in the caller's deterministic locator namespace.
    pub text_hash: &'a str,
    pub embedding: &'a [f32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub source_id: String,
    pub revision: String,
    pub locator: String,
    pub text_hash: String,
    pub similarity: f64,
}

/// Creates an isolated experimental schema. Never changes SearchService's
/// existing SQLite user_version, Tantivy schema or background worker.
pub fn prepare(connection: &Connection) -> Result<(), ProbeError> {
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS semantic_probe_models (
             model_id TEXT PRIMARY KEY,
             dimension INTEGER NOT NULL CHECK(dimension > 0 AND dimension <= 4096)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS semantic_probe_sources (
             scope_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             active_revision TEXT NOT NULL,
             PRIMARY KEY(scope_id, source_id)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS semantic_probe_chunks (
             scope_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             revision TEXT NOT NULL,
             locator TEXT NOT NULL,
             model_id TEXT NOT NULL,
             text_hash TEXT NOT NULL,
             embedding BLOB NOT NULL,
             PRIMARY KEY(scope_id, source_id, revision, locator, model_id),
             FOREIGN KEY(scope_id, source_id)
                 REFERENCES semantic_probe_sources(scope_id, source_id) ON DELETE CASCADE,
             FOREIGN KEY(model_id) REFERENCES semantic_probe_models(model_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS semantic_probe_lookup
             ON semantic_probe_chunks(scope_id, source_id, model_id);",
    )?;
    Ok(())
}

/// Call as soon as a newer source revision is observed, BEFORE asynchronous
/// embedding generation. Stale vectors remain reconstructible on disk but
/// immediately disappear from search via the revision join.
pub fn mark_revision(
    connection: &Connection,
    scope_id: &str,
    source_id: &str,
    revision: &str,
) -> Result<(), ProbeError> {
    key(scope_id)?;
    key(source_id)?;
    digest(revision)?;
    connection.execute(
        "INSERT INTO semantic_probe_sources(scope_id, source_id, active_revision)
         VALUES(?1, ?2, ?3)
         ON CONFLICT(scope_id, source_id)
         DO UPDATE SET active_revision=excluded.active_revision",
        params![scope_id, source_id, revision],
    )?;
    Ok(())
}

/// Replace one model's chunks for a source atomically. Never activates a
/// revision: a trusted caller must first observe/mark that revision.
pub fn replace_chunks(
    connection: &mut Connection,
    scope_id: &str,
    source_id: &str,
    revision: &str,
    model_id: &str,
    dimension: usize,
    chunks: &[Chunk<'_>],
) -> Result<(), ProbeError> {
    key(scope_id)?;
    key(source_id)?;
    key(model_id)?;
    digest(revision)?;
    if !(1..=MAX_DIMENSION).contains(&dimension) {
        return Err(ProbeError::Invalid("Invalid model dimension"));
    }
    if chunks.len() > MAX_CHUNKS_PER_SOURCE {
        return Err(ProbeError::Invalid("Too many chunks for one source"));
    }
    // Validate the full batch before touching the database.
    let vectors = chunks
        .iter()
        .map(|chunk| {
            key(chunk.locator)?;
            digest(chunk.text_hash)?;
            encode(chunk.embedding, dimension)
        })
        .collect::<Result<Vec<_>, ProbeError>>()?;

    let transaction = connection.transaction()?;
    let current: Option<String> = transaction
        .query_row(
            "SELECT active_revision FROM semantic_probe_sources
             WHERE scope_id=?1 AND source_id=?2",
            params![scope_id, source_id],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() != Some(revision) {
        return Err(ProbeError::Invalid("Source revision is not current"));
    }
    transaction.execute(
        "INSERT OR IGNORE INTO semantic_probe_models(model_id, dimension) VALUES(?1, ?2)",
        params![model_id, dimension as i64],
    )?;
    let existing: i64 = transaction.query_row(
        "SELECT dimension FROM semantic_probe_models WHERE model_id=?1",
        params![model_id],
        |row| row.get(0),
    )?;
    if existing != dimension as i64 {
        return Err(ProbeError::Invalid("Model id cannot change its dimension"));
    }
    transaction.execute(
        "DELETE FROM semantic_probe_chunks
         WHERE scope_id=?1 AND source_id=?2 AND model_id=?3",
        params![scope_id, source_id, model_id],
    )?;
    {
        let mut insert = transaction.prepare(
            "INSERT INTO semantic_probe_chunks
                (scope_id, source_id, revision, locator, model_id, text_hash, embedding)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
            insert.execute(params![
                scope_id,
                source_id,
                revision,
                chunk.locator,
                model_id,
                chunk.text_hash,
                vector,
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// `allowed_source_ids` MUST be freshly resolved from live permissions by
/// trusted host code. This primitive is not an authorization decision. It
/// only scores explicitly allowed sources in one scope with one model.
pub fn search(
    connection: &Connection,
    scope_id: &str,
    allowed_source_ids: &[&str],
    model_id: &str,
    query: &[f32],
    limit: usize,
) -> Result<Vec<Hit>, ProbeError> {
    key(scope_id)?;
    key(model_id)?;
    if allowed_source_ids.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let dimension: Option<i64> = connection
        .query_row(
            "SELECT dimension FROM semantic_probe_models WHERE model_id=?1",
            params![model_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(dimension) = dimension else {
        return Ok(Vec::new());
    };
    if dimension <= 0 || dimension as usize > MAX_DIMENSION {
        return Err(ProbeError::CorruptVector);
    }
    let normalized = normalized(query, dimension as usize)?;
    let permitted = allowed_source_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut statement = connection.prepare(
        "SELECT c.source_id, c.revision, c.locator, c.text_hash, c.embedding
         FROM semantic_probe_chunks c
         JOIN semantic_probe_sources s
           ON s.scope_id=c.scope_id AND s.source_id=c.source_id
          AND s.active_revision=c.revision
         WHERE c.scope_id=?1 AND c.source_id=?2 AND c.model_id=?3",
    )?;
    let mut scanned = 0usize;
    let mut hits = Vec::new();
    for source_id in permitted {
        key(source_id)?;
        let rows = statement.query_map(params![scope_id, source_id, model_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })?;
        for row in rows {
            scanned += 1;
            if scanned > MAX_SCANNED_CHUNKS {
                return Err(ProbeError::ScanBudgetExceeded);
            }
            let (source_id, revision, locator, text_hash, bytes) = row?;
            let vector = decode(&bytes, dimension as usize)?;
            let similarity = normalized
                .iter()
                .zip(vector.iter())
                .map(|(a, b)| a * f64::from(*b))
                .sum::<f64>();
            hits.push(Hit { source_id, revision, locator, text_hash, similarity });
        }
    }
    hits.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| a.source_id.cmp(&b.source_id))
            .then_with(|| a.locator.cmp(&b.locator))
    });
    hits.truncate(limit.min(MAX_RESULTS));
    Ok(hits)
}

fn key(value: &str) -> Result<(), ProbeError> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ProbeError::Invalid("Invalid identity or locator"));
    }
    Ok(())
}

fn digest(value: &str) -> Result<(), ProbeError> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(ProbeError::Invalid("Expected lowercase 64-character content hash"));
    }
    Ok(())
}

fn normalized(values: &[f32], dimension: usize) -> Result<Vec<f64>, ProbeError> {
    if values.len() != dimension || values.iter().any(|v| !v.is_finite()) {
        return Err(ProbeError::Invalid("Vector dimension or finite-value check failed"));
    }
    let norm = values.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return Err(ProbeError::Invalid("Zero or invalid embedding"));
    }
    Ok(values.iter().map(|v| f64::from(*v) / norm).collect())
}

fn encode(values: &[f32], dimension: usize) -> Result<Vec<u8>, ProbeError> {
    let numbers = normalized(values, dimension)?;
    let mut result = Vec::with_capacity(dimension * 4);
    for value in numbers {
        result.extend_from_slice(&(value as f32).to_le_bytes());
    }
    Ok(result)
}

fn decode(bytes: &[u8], dimension: usize) -> Result<Vec<f32>, ProbeError> {
    if bytes.len() != dimension * 4 {
        return Err(ProbeError::CorruptVector);
    }
    let values = bytes
        .chunks_exact(4)
        .map(|raw| f32::from_le_bytes(raw.try_into().expect("four bytes")))
        .collect::<Vec<_>>();
    if values.iter().any(|v| !v.is_finite()) {
        return Err(ProbeError::CorruptVector);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(text: &str) -> String {
        blake3::hash(text.as_bytes()).to_hex().to_string()
    }

    fn insert(
        connection: &mut Connection,
        scope: &str,
        source: &str,
        revision: &str,
        vector: &[f32],
    ) {
        let text_hash = hash("fragment");
        mark_revision(connection, scope, source, revision).unwrap();
        replace_chunks(
            connection,
            scope,
            source,
            revision,
            "local-test-model@1",
            2,
            &[Chunk { locator: "heading:1", text_hash: &text_hash, embedding: vector }],
        )
        .unwrap();
    }

    #[test]
    fn only_allowed_sources_and_scope_are_returned() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let rev = hash("v1");
        insert(&mut db, "vault-a", "allowed", &rev, &[1.0, 0.0]);
        insert(&mut db, "vault-a", "private", &rev, &[1.0, 0.0]);
        insert(&mut db, "vault-b", "allowed", &rev, &[1.0, 0.0]);
        let hits = search(&db, "vault-a", &["allowed"], "local-test-model@1", &[1.0, 0.0], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_id, "allowed");
        assert_eq!(hits[0].revision, rev);
        assert!(search(&db, "vault-a", &[], "local-test-model@1", &[1.0, 0.0], 10).unwrap().is_empty());
    }

    #[test]
    fn new_revision_hides_stale_vectors_before_reembedding() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let v1 = hash("old");
        let v2 = hash("new");
        insert(&mut db, "one", "note", &v1, &[1.0, 0.0]);
        mark_revision(&db, "one", "note", &v2).unwrap();
        assert!(search(&db, "one", &["note"], "local-test-model@1", &[1.0, 0.0], 10).unwrap().is_empty());
        let old = replace_chunks(&mut db, "one", "note", &v1, "local-test-model@1", 2, &[]);
        assert!(matches!(old, Err(ProbeError::Invalid(_))));
        insert(&mut db, "one", "note", &v2, &[0.0, 1.0]);
        let hits = search(&db, "one", &["note"], "local-test-model@1", &[0.0, 1.0], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].revision, v2);
    }

    #[test]
    fn rejects_wrong_dimensions_and_nan_without_losing_saved_vector() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let rev = hash("v1");
        insert(&mut db, "one", "note", &rev, &[1.0, 0.0]);
        let good = hash("fragment");
        assert!(replace_chunks(
            &mut db, "one", "note", &rev, "local-test-model@1", 3,
            &[Chunk { locator: "heading:1", text_hash: &good, embedding: &[1.0, 0.0, 0.0] }]
        ).is_err());
        assert!(replace_chunks(
            &mut db, "one", "note", &rev, "local-test-model@1", 2,
            &[Chunk { locator: "heading:1", text_hash: &good, embedding: &[f32::NAN, 0.0] }]
        ).is_err());
        assert!(search(&db, "one", &["note"], "local-test-model@1", &[0.0, 0.0], 5).is_err());
        assert_eq!(
            search(&db, "one", &["note"], "local-test-model@1", &[1.0, 0.0], 5).unwrap().len(),
            1
        );
    }

    #[test]
    fn persistent_cache_reopens_and_replacement_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("semantic-probe.sqlite");
        let rev = hash("v1");
        {
            let mut db = Connection::open(&path).unwrap();
            prepare(&db).unwrap();
            insert(&mut db, "one", "note", &rev, &[1.0, 0.0]);
            insert(&mut db, "one", "note", &rev, &[1.0, 0.0]);
        }
        let db = Connection::open(&path).unwrap();
        prepare(&db).unwrap();
        let hits = search(&db, "one", &["note"], "local-test-model@1", &[1.0, 0.0], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!((hits[0].similarity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn malformed_and_corrupt_data_fail_closed() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let rev = hash("one");
        assert!(mark_revision(&db, "one", "note", "bad-revision").is_err());
        insert(&mut db, "one", "note", &rev, &[1.0, 0.0]);
        db.execute(
            "UPDATE semantic_probe_chunks SET embedding=x'00000000'
             WHERE scope_id='one' AND source_id='note'",
            [],
        ).unwrap();
        assert!(matches!(
            search(&db, "one", &["note"], "local-test-model@1", &[1.0, 0.0], 10),
            Err(ProbeError::CorruptVector)
        ));
    }
}
