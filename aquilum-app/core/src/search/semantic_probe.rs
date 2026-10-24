//! Isolated multiversion vector-index experiment. Not a source of truth.
//!
//! Sync Core or a trusted external-source adapter owns revision ancestry,
//! selected heads, content verification, and authorization. This module stores
//! disposable embeddings for immutable source revisions. It never elects a
//! "latest" revision, modifies files, or exposes an MCP / native search route.
//! A local generation fences delayed workers; it is NOT a Sync RevisionId,
//! a causal clock, or an S3 manifest generation. Old v1 probe tables are not
//! read or migrated: this experimental v2 cache can be rebuilt.
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;
use std::fmt;

const MAX_DIMENSION: usize = 4096;
const MAX_CHUNKS_PER_REVISION: usize = 1024;
const MAX_SCANNED_CHUNKS: usize = 8192;
const MAX_RESULTS: usize = 50;
const MAX_TARGETS: usize = 128;
const MAX_REVISIONS_PER_TARGET: usize = 32;

#[derive(Debug)]
pub enum ProbeError {
    Invalid(&'static str),
    CorruptVector,
    ScanBudgetExceeded,
    StaleSource,
    AccessDenied,
    RevisionCollision,
    UnknownModel,
    Database(rusqlite::Error),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(why) => f.write_str(why),
            Self::CorruptVector => f.write_str("Stored vector is corrupt"),
            Self::ScanBudgetExceeded => f.write_str("Semantic probe scan budget exceeded"),
            Self::StaleSource => f.write_str("Local source generation or access changed"),
            Self::AccessDenied => f.write_str("Semantic search permission denied"),
            Self::RevisionCollision => f.write_str("Immutable revision has different source bytes"),
            Self::UnknownModel => f.write_str("Requested embedding model has no index"),
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

/// Stable locator within an immutable revision and hash of extracted text.
pub struct Chunk<'a> {
    pub locator: &'a str,
    pub text_hash: &'a str,
    pub embedding: &'a [f32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionKind {
    Content,
    Tombstone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionRef<'a> {
    /// Sync RevisionId as a string, or provider-native opaque version.
    pub id: &'a str,
    pub kind: RevisionKind,
}

/// The trusted caller must resolve this from Sync DAG or the source owner.
/// No "current" selector can be derived from arrival order or index data.
pub enum RevisionSelection<'a> {
    Committed(RevisionRef<'a>),
    Conflicted(&'a [RevisionRef<'a>]),
    Historical(RevisionRef<'a>),
}

/// Read-only resolver for a Sync document. It does not change DAG heads,
/// publish files or infer ordering from timestamps. The caller must separately
/// establish a trusted DAG snapshot and read grants for this document.
/// Pending ancestry for this document prevents an incomplete "current" view.
pub fn resolve_sync_revisions(
    dag: &crate::sync::RevisionDag,
    document: crate::sync::DocumentId,
    requested_history: Option<crate::sync::RevisionId>,
) -> Result<OwnedSelection, ProbeError> {
    use crate::sync::RevisionOperation;
    let refs = if let Some(revision) = requested_history {
        let record = dag.get(revision).ok_or(ProbeError::StaleSource)?;
        if record.document_id != document {
            return Err(ProbeError::Invalid("Revision belongs to another document"));
        }
        vec![record]
    } else {
        if dag.has_pending_for_document(document) {
            return Err(ProbeError::StaleSource);
        }
        let heads = dag.heads(document);
        if heads.is_empty() {
            return Err(ProbeError::StaleSource);
        }
        heads.into_iter().map(|id| dag.get(id).expect("verified head exists")).collect()
    };
    let mut revisions = Vec::with_capacity(refs.len());
    for record in refs {
        revisions.push(OwnedRevision {
            id: record.id.to_string(),
            kind: if matches!(&record.operation, RevisionOperation::Tombstone { .. }) {
                RevisionKind::Tombstone
            } else {
                RevisionKind::Content
            },
            source_hash: match &record.operation {
                RevisionOperation::Content { blob_hash, .. }
                | RevisionOperation::Rename { blob_hash, .. } => Some(blob_hash.clone()),
                RevisionOperation::Tombstone { .. } => None,
            },
        });
    }
    Ok(OwnedSelection {
        document_id: document.to_string(),
        historical: requested_history.is_some(),
        conflict: revisions.len() > 1,
        revisions,
    })
}

/// Owned, verified revision IDs and expected immutable content hashes.
/// Kept distinct from the low-level external-provider selection API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedRevision {
    id: String,
    kind: RevisionKind,
    source_hash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedSelection {
    document_id: String,
    historical: bool,
    conflict: bool,
    revisions: Vec<OwnedRevision>,
}

/// Only the verified DAG resolver can construct this selection. Trusted
/// host code can retrieve its typed identity, not synthesize another.
impl OwnedSelection {
    pub(crate) fn sync_document_id(&self) -> Result<crate::sync::DocumentId, ProbeError> {
        self.document_id.parse().map_err(|_| ProbeError::StaleSource)
    }
}

/// Search the resolved Sync revisions, enforcing their original blob hashes.
/// The outer host must check live permissions both before and after this call.
/// This is NOT a public RPC; only trusted callers should be given a DAG.
pub fn search_sync_verified(
    connection: &Connection,
    scope_id: &str,
    expected_generation: u64,
    selection: &OwnedSelection,
    model_id: &str,
    query: &[f32],
    limit: usize,
) -> Result<SearchResult, ProbeError> {
    if selection.revisions.is_empty() {
        return Err(ProbeError::StaleSource);
    }
    let refs: Vec<RevisionRef<'_>> = selection.revisions.iter().map(|r| RevisionRef {
        id: &r.id,
        kind: r.kind,
    }).collect();
    let selected = if selection.historical {
        RevisionSelection::Historical(refs[0])
    } else if selection.conflict {
        RevisionSelection::Conflicted(&refs)
    } else {
        RevisionSelection::Committed(refs[0])
    };
    let result = search(connection, scope_id, &[SearchTarget {
        source_id: &selection.document_id,
        expected_generation,
        selection: selected,
    }], model_id, query, limit)?;
    // Validate immutable content provenance for EACH indexed revision,
    // including revisions whose vector index is valid but has zero chunks.
    // Checking hits alone would miss corrupted records of empty documents.
    for revision in selection.revisions.iter().filter(|r| r.kind == RevisionKind::Content) {
        let stored: Option<String> = connection.query_row(
            "SELECT source_hash FROM semantic_probe_v2_revisions
             WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3",
            params![scope_id, selection.document_id, revision.id],
            |row| row.get(0),
        ).optional()?;
        if let Some(stored) = stored {
            if revision.source_hash.as_deref() != Some(stored.as_str()) {
                return Err(ProbeError::RevisionCollision);
            }
        }
    }
    Ok(result)
}

/// A conservative host-side binding for a CURRENT, single-head local note.
/// This does not conflate the UI NoteId with the Sync DocumentId. Instead the
/// host supplies a pinned note path, its verified on-disk BLAKE3 and a complete
/// trusted DAG; cross-document aliases, pending revisions and conflicts deny.
/// Historical revisions require a separate explicit grant not provided here.
pub(crate) fn bind_current_note_to_sync(
    dag: &crate::sync::RevisionDag,
    relative_path: &str,
    observed_content_hash: &str,
) -> Result<OwnedSelection, ProbeError> {
    use crate::sync::{RevisionOperation, SyncRelativePath};
    let path = SyncRelativePath::parse(relative_path)
        .map_err(|_| ProbeError::AccessDenied)?;
    if path.as_str() != relative_path {
        return Err(ProbeError::AccessDenied);
    }
    digest(observed_content_hash)?;
    let candidates = dag.current_path_claimants(&path);
    if candidates.len() != 1 {
        return Err(ProbeError::AccessDenied);
    }
    let document = *candidates.iter().next().ok_or(ProbeError::AccessDenied)?;
    let selection = resolve_sync_revisions(dag, document, None)?;
    if selection.historical || selection.conflict || selection.revisions.len() != 1 {
        return Err(ProbeError::AccessDenied);
    }
    let revision_id: crate::sync::RevisionId = selection.revisions[0].id.parse()
        .map_err(|_| ProbeError::AccessDenied)?;
    let record = dag.get(revision_id).ok_or(ProbeError::StaleSource)?;
    let current_path = match &record.operation {
        RevisionOperation::Content { path, .. } => path,
        RevisionOperation::Rename { to, .. } => to,
        RevisionOperation::Tombstone { .. } => return Err(ProbeError::AccessDenied),
    };
    if current_path != &path || selection.revisions[0].source_hash.as_deref()
        != Some(observed_content_hash)
    {
        return Err(ProbeError::AccessDenied);
    }
    Ok(selection)
}

/// Restricted, host-only plugin path. Uses the EXISTING PermissionBroker grant
/// for NotesSearch rather than inventing a second permission system.
///
/// The trusted host MUST independently bind the host-indexed NoteResource to
/// expected_document and the registered vault scope; neither NoteId nor an
/// arbitrary path is automatically a Sync DocumentId. This method checks that
/// scope_id is the registered plugin vault, that the owner's selected document
/// still matches expected_document, and that a live lease still authorizes the
/// note before and after the search.
///
/// No untrusted plugin arguments, remote agents or MCP routes may call this.
/// It returns metadata only, not note bytes. The RPC/response boundary must
/// revalidate its own authorization before any bytes are actually transmitted.
pub(crate) fn search_sync_with_plugin_grant<Owner, Context>(
    connection: &Connection,
    broker: &crate::plugins::PermissionBroker,
    plugin: crate::plugins::PluginInstanceHandle,
    note: &crate::plugins::NoteResource,
    expected_document: crate::sync::DocumentId,
    scope_id: &str,
    model_id: &str,
    query: &[f32],
    limit: usize,
    mut live_context: Context,
    mut observe_owner: Owner,
) -> Result<SearchResult, ProbeError>
where
    Owner: FnMut() -> Result<ReadObservation, ProbeError>,
    Context: FnMut() -> crate::plugins::HostContext,
{
    use crate::plugins::{Capability, Resource};
    // The isolated index's scope is the registered vault ID (not a
    // user-chosen project label or arbitrary search namespace).
    if scope_id != note.vault.as_uuid().to_string() {
        return Err(ProbeError::AccessDenied);
    }
    let resource = Resource::Note(note.clone());
    let lease = broker
        .authorize(plugin, live_context(), Capability::NotesSearch, &resource)
        .map_err(|_| ProbeError::AccessDenied)?;
    search_sync_revalidated(connection, scope_id, model_id, query, limit, || {
        broker
            .revalidate_lease(&lease, live_context())
            .map_err(|_| ProbeError::AccessDenied)?;
        let observed = observe_owner()?;
        // A grant to the CURRENT note does not imply read access to prior
        // snapshots or either branch of a sync conflict. Those require a
        // separate, explicit historical/conflict authorization route.
        if observed.selection.document_id != expected_document.to_string()
            || observed.selection.historical
            || observed.selection.conflict
            || observed.selection.revisions.len() != 1
            || observed.selection.revisions[0].kind != RevisionKind::Content
        {
            return Err(ProbeError::AccessDenied);
        }
        // The lease is independently revalidated by PermissionBroker.
        // The owner callback's epoch is only an additional snapshot fence,
        // never a replacement for the permission grant.
        broker
            .revalidate_lease(&lease, live_context())
            .map_err(|_| ProbeError::AccessDenied)?;
        Ok(observed)
    })
}

/// A snapshot provided by the trusted host. Epochs MUST be monotonic and
/// checked against real authorization and a synchronized source snapshot.
/// This struct neither grants access nor verifies a caller's identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadObservation {
    pub(crate) grant_epoch: u64,
    pub(crate) owner_epoch: u64,
    pub(crate) index_generation: u64,
    pub(crate) selection: OwnedSelection,
}

/// Execute an isolated search with a freshly revalidated host observation.
///
/// The host callback must check the live grant and obtain its monotonic epoch,
/// then derive revisions from a trusted Sync DAG or provider snapshot. It runs
/// once before scoring and once before the result is returned. Any revocation,
/// permission change, source change, or read failure must return an error.
/// The final RPC/response transport STILL needs its own admission protection:
/// another revocation can happen immediately after this function returns.
pub(crate) fn search_sync_revalidated<F>(
    connection: &Connection,
    scope_id: &str,
    model_id: &str,
    query: &[f32],
    limit: usize,
    mut observe: F,
) -> Result<SearchResult, ProbeError>
where
    F: FnMut() -> Result<ReadObservation, ProbeError>,
{
    let before = observe()?;
    if before.grant_epoch == 0 || before.owner_epoch == 0 || before.index_generation == 0 {
        return Err(ProbeError::StaleSource);
    }
    let result = search_sync_verified(
        connection, scope_id, before.index_generation, &before.selection,
        model_id, query, limit,
    )?;
    let after = observe()?;
    if before != after {
        return Err(ProbeError::StaleSource);
    }
    Ok(result)
}

/// Only authorized sources/revisions may be supplied by the trusted host.
/// The generation is a LOCAL stale-worker/revocation fence, not revision order.
pub struct SearchTarget<'a> {
    pub source_id: &'a str,
    pub expected_generation: u64,
    pub selection: RevisionSelection<'a>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRevision {
    pub source_id: String,
    pub revision_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub source_id: String,
    pub revision_id: String,
    pub source_hash: String,
    pub locator: String,
    pub text_hash: String,
    pub similarity: f64,
}

/// A partial index is not a "no results" answer. Conflicts and tombstones
/// are surfaced even when only one branch has cached vectors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResult {
    pub hits: Vec<Hit>,
    pub missing: Vec<SourceRevision>,
    pub tombstones: Vec<SourceRevision>,
    pub conflicts: Vec<String>,
}

/// All schema objects use a new namespace. The v1 cache remains untouched;
/// this laboratory neither migrates it nor changes the main search database.
pub fn prepare(connection: &Connection) -> Result<(), ProbeError> {
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS semantic_probe_v2_models (
             model_id TEXT PRIMARY KEY,
             dimension INTEGER NOT NULL CHECK(dimension > 0 AND dimension <= 4096)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS semantic_probe_v2_sources (
             scope_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             generation INTEGER NOT NULL CHECK(generation > 0),
             revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0, 1)),
             PRIMARY KEY(scope_id, source_id)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS semantic_probe_v2_revisions (
             scope_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             revision_id TEXT NOT NULL,
             source_hash TEXT NOT NULL,
             PRIMARY KEY(scope_id, source_id, revision_id),
             FOREIGN KEY(scope_id, source_id)
                 REFERENCES semantic_probe_v2_sources(scope_id, source_id)
                 ON DELETE CASCADE
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS semantic_probe_v2_ready (
             scope_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             revision_id TEXT NOT NULL,
             model_id TEXT NOT NULL,
             PRIMARY KEY(scope_id, source_id, revision_id, model_id),
             FOREIGN KEY(scope_id, source_id, revision_id)
                 REFERENCES semantic_probe_v2_revisions(scope_id, source_id, revision_id)
                 ON DELETE CASCADE,
             FOREIGN KEY(model_id)
                 REFERENCES semantic_probe_v2_models(model_id)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS semantic_probe_v2_chunks (
             scope_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             revision_id TEXT NOT NULL,
             model_id TEXT NOT NULL,
             locator TEXT NOT NULL,
             text_hash TEXT NOT NULL,
             embedding BLOB NOT NULL,
             embedding_hash TEXT NOT NULL,
             PRIMARY KEY(scope_id, source_id, revision_id, model_id, locator),
             FOREIGN KEY(scope_id, source_id, revision_id, model_id)
                 REFERENCES semantic_probe_v2_ready(scope_id, source_id, revision_id, model_id)
                 ON DELETE CASCADE
         ) WITHOUT ROWID;",
    )?;
    Ok(())
}

/// One-time admission for a trusted source, not a grant of read permission.
/// A revoked source remains tombstoned and cannot be silently re-enrolled.
pub fn register_source(
    connection: &Connection,
    scope_id: &str,
    source_id: &str,
) -> Result<u64, ProbeError> {
    key(scope_id)?;
    key(source_id)?;
    let inserted = connection.execute(
        "INSERT OR IGNORE INTO semantic_probe_v2_sources
             (scope_id, source_id, generation)
         VALUES(?1, ?2, 1)",
        params![scope_id, source_id],
    )?;
    if inserted != 1 {
        return Err(ProbeError::StaleSource);
    }
    Ok(1)
}

/// Fence previously started indexing jobs when a trusted owner reports a new
/// source snapshot or scope transition. Does not select any revision.
pub fn fence_source(
    connection: &Connection,
    scope_id: &str,
    source_id: &str,
    expected_generation: u64,
) -> Result<u64, ProbeError> {
    key(scope_id)?;
    key(source_id)?;
    let generation = db_generation(expected_generation)?;
    let changed = connection.execute(
        "UPDATE semantic_probe_v2_sources SET generation=generation+1
         WHERE scope_id=?1 AND source_id=?2 AND generation=?3 AND revoked=0",
        params![scope_id, source_id, generation],
    )?;
    if changed != 1 {
        return Err(ProbeError::StaleSource);
    }
    Ok(expected_generation + 1)
}

/// Revocation invalidates local cached vectors for ALL revisions, including
/// history, and leaves a tombstone to block delayed jobs/re-enrollment.
/// This is not an immutable Sync deletion event or physical disk erasure.
pub fn revoke_source(
    connection: &mut Connection,
    scope_id: &str,
    source_id: &str,
    expected_generation: u64,
) -> Result<u64, ProbeError> {
    key(scope_id)?;
    key(source_id)?;
    let generation = db_generation(expected_generation)?;
    let transaction = connection.transaction()?;
    let changed = transaction.execute(
        "UPDATE semantic_probe_v2_sources
         SET revoked=1, generation=generation+1
         WHERE scope_id=?1 AND source_id=?2 AND generation=?3 AND revoked=0",
        params![scope_id, source_id, generation],
    )?;
    if changed != 1 {
        return Err(ProbeError::StaleSource);
    }
    transaction.execute(
        "DELETE FROM semantic_probe_v2_revisions
         WHERE scope_id=?1 AND source_id=?2",
        params![scope_id, source_id],
    )?;
    transaction.commit()?;
    Ok(expected_generation + 1)
}

/// Store or replace vectors for ONE exact immutable revision and model.
/// Other revisions and models are untouched. The source hash guards against
/// accidental reuse of the same revision ID for different file content.
/// The caller must have independently verified all source bytes/ancestry.
pub fn replace_revision_chunks(
    connection: &mut Connection,
    scope_id: &str,
    source_id: &str,
    expected_generation: u64,
    revision_id: &str,
    source_hash: &str,
    model_id: &str,
    dimension: usize,
    chunks: &[Chunk<'_>],
) -> Result<(), ProbeError> {
    key(scope_id)?;
    key(source_id)?;
    key(revision_id)?;
    digest(source_hash)?;
    key(model_id)?;
    let generation = db_generation(expected_generation)?;
    if !(1..=MAX_DIMENSION).contains(&dimension) {
        return Err(ProbeError::Invalid("Invalid model dimension"));
    }
    if chunks.len() > MAX_CHUNKS_PER_REVISION {
        return Err(ProbeError::Invalid("Too many chunks in one revision"));
    }
    let mut locators = BTreeSet::new();
    let encoded = chunks.iter().map(|chunk| {
        key(chunk.locator)?;
        digest(chunk.text_hash)?;
        if !locators.insert(chunk.locator) {
            return Err(ProbeError::Invalid("Duplicate chunk locator"));
        }
        encode(chunk.embedding, dimension)
    }).collect::<Result<Vec<_>, ProbeError>>()?;

    let transaction = connection.transaction()?;
    let active: Option<i64> = transaction.query_row(
        "SELECT generation FROM semantic_probe_v2_sources
         WHERE scope_id=?1 AND source_id=?2 AND revoked=0",
        params![scope_id, source_id],
        |row| row.get(0),
    ).optional()?;
    if active != Some(generation) {
        return Err(ProbeError::StaleSource);
    }

    transaction.execute(
        "INSERT OR IGNORE INTO semantic_probe_v2_models(model_id, dimension)
         VALUES(?1, ?2)",
        params![model_id, dimension as i64],
    )?;
    let model_dim: i64 = transaction.query_row(
        "SELECT dimension FROM semantic_probe_v2_models WHERE model_id=?1",
        params![model_id],
        |row| row.get(0),
    )?;
    if model_dim != dimension as i64 {
        return Err(ProbeError::Invalid("Model id changed dimension"));
    }

    transaction.execute(
        "INSERT OR IGNORE INTO semantic_probe_v2_revisions
             (scope_id, source_id, revision_id, source_hash)
         VALUES(?1, ?2, ?3, ?4)",
        params![scope_id, source_id, revision_id, source_hash],
    )?;
    let stored_hash: String = transaction.query_row(
        "SELECT source_hash FROM semantic_probe_v2_revisions
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3",
        params![scope_id, source_id, revision_id],
        |row| row.get(0),
    )?;
    if stored_hash != source_hash {
        return Err(ProbeError::RevisionCollision);
    }

    transaction.execute(
        "INSERT OR IGNORE INTO semantic_probe_v2_ready
             (scope_id, source_id, revision_id, model_id)
         VALUES(?1, ?2, ?3, ?4)",
        params![scope_id, source_id, revision_id, model_id],
    )?;
    transaction.execute(
        "DELETE FROM semantic_probe_v2_chunks
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND model_id=?4",
        params![scope_id, source_id, revision_id, model_id],
    )?;
    {
        let mut insert = transaction.prepare(
            "INSERT INTO semantic_probe_v2_chunks
                (scope_id, source_id, revision_id, model_id, locator, text_hash,
                 embedding, embedding_hash)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for (chunk, bytes) in chunks.iter().zip(encoded.iter()) {
            let vector_hash = blake3::hash(bytes).to_hex().to_string();
            insert.execute(params![
                scope_id,
                source_id,
                revision_id,
                model_id,
                chunk.locator,
                chunk.text_hash,
                bytes,
                vector_hash,
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// Evict only one cached revision/model. Does not delete the revision identity
/// or any original bytes; historical search can return "missing" and rebuild.
pub fn evict_cached_revision(
    connection: &Connection,
    scope_id: &str,
    source_id: &str,
    expected_generation: u64,
    revision_id: &str,
    model_id: &str,
) -> Result<(), ProbeError> {
    key(scope_id)?;
    key(source_id)?;
    key(revision_id)?;
    key(model_id)?;
    let generation = db_generation(expected_generation)?;
    let active: Option<i64> = connection.query_row(
        "SELECT generation FROM semantic_probe_v2_sources
         WHERE scope_id=?1 AND source_id=?2 AND revoked=0",
        params![scope_id, source_id],
        |row| row.get(0),
    ).optional()?;
    if active != Some(generation) {
        return Err(ProbeError::StaleSource);
    }
    connection.execute(
        "DELETE FROM semantic_probe_v2_ready
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND model_id=?4",
        params![scope_id, source_id, revision_id, model_id],
    )?;
    Ok(())
}

/// Search ONLY exact caller-selected revisions. The host must first verify
/// live read permissions, resolve Sync DAG heads or owner-native versions,
/// and recheck grants before releasing results. No automatic latest/version
/// election is possible through this API. Unknown versions are "missing".
pub fn search(
    connection: &Connection,
    scope_id: &str,
    targets: &[SearchTarget<'_>],
    model_id: &str,
    query: &[f32],
    limit: usize,
) -> Result<SearchResult, ProbeError> {
    key(scope_id)?;
    key(model_id)?;
    if targets.len() > MAX_TARGETS {
        return Err(ProbeError::Invalid("Too many source targets"));
    }
    if targets.is_empty() || limit == 0 {
        return Ok(SearchResult::default());
    }
    let dimension: Option<i64> = connection.query_row(
        "SELECT dimension FROM semantic_probe_v2_models WHERE model_id=?1",
        params![model_id],
        |row| row.get(0),
    ).optional()?;
    let dimension = dimension.ok_or(ProbeError::UnknownModel)?;
    if dimension <= 0 || dimension as usize > MAX_DIMENSION {
        return Err(ProbeError::CorruptVector);
    }
    let normalized = normalized(query, dimension as usize)?;
    let mut seen_sources = BTreeSet::new();
    let mut result = SearchResult::default();
    let mut scanned = 0usize;
    let mut statement = connection.prepare(
        "SELECT c.locator, c.text_hash, c.embedding, c.embedding_hash, r.source_hash
         FROM semantic_probe_v2_chunks c
         JOIN semantic_probe_v2_revisions r
           ON r.scope_id=c.scope_id AND r.source_id=c.source_id
          AND r.revision_id=c.revision_id
         WHERE c.scope_id=?1 AND c.source_id=?2 AND c.revision_id=?3
           AND c.model_id=?4",
    )?;
    for target in targets {
        key(target.source_id)?;
        let generation = db_generation(target.expected_generation)?;
        if !seen_sources.insert(target.source_id) {
            return Err(ProbeError::Invalid("Repeated source in search targets"));
        }
        let state: Option<(i64, i64)> = connection.query_row(
            "SELECT generation, revoked FROM semantic_probe_v2_sources
             WHERE scope_id=?1 AND source_id=?2",
            params![scope_id, target.source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        if state != Some((generation, 0)) {
            return Err(ProbeError::StaleSource);
        }
        let refs: Vec<RevisionRef<'_>> = match &target.selection {
            RevisionSelection::Committed(item) | RevisionSelection::Historical(item) => vec![*item],
            RevisionSelection::Conflicted(heads) => {
                if heads.len() < 2 {
                    return Err(ProbeError::Invalid("Conflict requires multiple heads"));
                }
                result.conflicts.push(target.source_id.to_owned());
                heads.to_vec()
            }
        };
        if refs.len() > MAX_REVISIONS_PER_TARGET {
            return Err(ProbeError::Invalid("Too many requested revisions"));
        }
        let mut seen_revisions = BTreeSet::new();
        for revision in refs {
            key(revision.id)?;
            if !seen_revisions.insert(revision.id) {
                return Err(ProbeError::Invalid("Duplicate revision in selection"));
            }
            let reference = SourceRevision {
                source_id: target.source_id.to_owned(),
                revision_id: revision.id.to_owned(),
            };
            if revision.kind == RevisionKind::Tombstone {
                result.tombstones.push(reference);
                continue;
            }
            let ready: Option<i64> = connection.query_row(
                "SELECT 1 FROM semantic_probe_v2_ready
                 WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND model_id=?4",
                params![scope_id, target.source_id, revision.id, model_id],
                |row| row.get(0),
            ).optional()?;
            if ready.is_none() {
                result.missing.push(reference);
                continue;
            }
            let rows = statement.query_map(
                params![scope_id, target.source_id, revision.id, model_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?;
            for row in rows {
                scanned += 1;
                if scanned > MAX_SCANNED_CHUNKS {
                    return Err(ProbeError::ScanBudgetExceeded);
                }
                let (locator, text_hash, bytes, vector_hash, source_hash) = row?;
                if blake3::hash(&bytes).to_hex().as_str() != vector_hash {
                    return Err(ProbeError::CorruptVector);
                }
                let vector = decode(&bytes, dimension as usize)?;
                let similarity = normalized.iter()
                    .zip(vector.iter())
                    .map(|(left, right)| left * f64::from(*right))
                    .sum::<f64>();
                result.hits.push(Hit {
                    source_id: target.source_id.to_owned(),
                    revision_id: revision.id.to_owned(),
                    source_hash,
                    locator,
                    text_hash,
                    similarity,
                });
            }
        }
    }
    result.hits.sort_by(|left, right| {
        right.similarity.total_cmp(&left.similarity)
            .then_with(|| left.source_id.cmp(&right.source_id))
            .then_with(|| left.revision_id.cmp(&right.revision_id))
            .then_with(|| left.locator.cmp(&right.locator))
    });
    result.hits.truncate(limit.min(MAX_RESULTS));
    Ok(result)
}

fn key(value: &str) -> Result<(), ProbeError> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ProbeError::Invalid("Invalid identity or locator"));
    }
    Ok(())
}

fn digest(value: &str) -> Result<(), ProbeError> {
    if value.len() != 64
        || !value.bytes().all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
    {
        return Err(ProbeError::Invalid("Expected lowercase BLAKE3 hex"));
    }
    Ok(())
}

fn db_generation(generation: u64) -> Result<i64, ProbeError> {
    if generation == 0 || generation >= i64::MAX as u64 {
        return Err(ProbeError::Invalid("Invalid local generation"));
    }
    Ok(generation as i64)
}

fn normalized(values: &[f32], dimension: usize) -> Result<Vec<f64>, ProbeError> {
    if values.len() != dimension || values.iter().any(|value| !value.is_finite()) {
        return Err(ProbeError::Invalid("Invalid embedding values/dimension"));
    }
    let norm = values.iter().map(|value| f64::from(*value).powi(2)).sum::<f64>().sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return Err(ProbeError::Invalid("Zero or invalid embedding"));
    }
    Ok(values.iter().map(|value| f64::from(*value) / norm).collect())
}

fn encode(values: &[f32], dimension: usize) -> Result<Vec<u8>, ProbeError> {
    let numbers = normalized(values, dimension)?;
    let mut bytes = Vec::with_capacity(dimension * 4);
    for number in numbers {
        bytes.extend_from_slice(&(number as f32).to_le_bytes());
    }
    Ok(bytes)
}

fn decode(bytes: &[u8], dimension: usize) -> Result<Vec<f32>, ProbeError> {
    if bytes.len() != dimension * 4 {
        return Err(ProbeError::CorruptVector);
    }
    let values: Vec<f32> = bytes.chunks_exact(4)
        .map(|part| f32::from_le_bytes(part.try_into().expect("four-byte chunk")))
        .collect();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(ProbeError::CorruptVector);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        DocumentId, RevisionDag, RevisionId, RevisionOperation, RevisionRecord, SyncRelativePath,
    };

    const MODEL: &str = "test-model@1";

    fn hash(text: &str) -> String {
        blake3::hash(text.as_bytes()).to_hex().to_string()
    }

    fn put(
        db: &mut Connection,
        scope: &str,
        source: &str,
        generation: u64,
        revision: &str,
        text: &str,
        vector: &[f32],
    ) {
        let body_hash = hash(text);
        let fragment_hash = hash("fragment");
        replace_revision_chunks(
            db, scope, source, generation, revision, &body_hash, MODEL, 2,
            &[Chunk {
                locator: "section:1",
                text_hash: &fragment_hash,
                embedding: vector,
            }],
        ).unwrap();
    }

    fn target<'a>(
        source_id: &'a str,
        generation: u64,
        selection: RevisionSelection<'a>,
    ) -> SearchTarget<'a> {
        SearchTarget { source_id, expected_generation: generation, selection }
    }

    fn content<'a>(id: &'a str) -> RevisionRef<'a> {
        RevisionRef { id, kind: RevisionKind::Content }
    }

    fn current<'a>(id: &'a str) -> RevisionSelection<'a> {
        RevisionSelection::Committed(content(id))
    }

    fn history<'a>(id: &'a str) -> RevisionSelection<'a> {
        RevisionSelection::Historical(content(id))
    }

    fn lookup(
        db: &Connection,
        scope: &str,
        source: &str,
        generation: u64,
        selection: RevisionSelection<'_>,
    ) -> Result<SearchResult, ProbeError> {
        search(db, scope, &[target(source, generation, selection)], MODEL, &[1.0, 0.0], 10)
    }

    #[test]
    fn resolver_rejects_cross_document_and_pending_head() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let other = DocumentId::new();
        let current = add_dag_revision(&mut dag, doc, vec![], "valid");
        let unrelated = add_dag_revision(&mut dag, other, vec![], "other");
        assert!(resolve_sync_revisions(&dag, doc, Some(unrelated)).is_err());
        let s = resolve_sync_revisions(&dag, doc, None).unwrap();
        assert_eq!(s.revisions[0].id, current.to_string());
        let delayed = RevisionId::new();
        let not_here = RevisionId::new();
        dag.ingest(RevisionRecord {
            id: delayed,
            document_id: doc,
            parents: vec![not_here],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/claim.md").unwrap(),
                blob_hash: hash("child"),
            },
        }).unwrap();
        assert!(matches!(
            resolve_sync_revisions(&dag, doc, None),
            Err(ProbeError::StaleSource)
        ));
        assert!(resolve_sync_revisions(&dag, other, None).is_ok());
    }

    #[test]
    fn verified_search_rejects_cached_hash_mismatch_and_keeps_history() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let rev = add_dag_revision(&mut dag, doc, vec![], "correct");
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let id = doc.to_string();
        register_source(&db, "vault", &id).unwrap();
        put(&mut db, "vault", &id, 1, &rev.to_string(), "wrong", &[1.0, 0.0]);
        let selected = resolve_sync_revisions(&dag, doc, None).unwrap();
        assert!(matches!(
            search_sync_verified(&db, "vault", 1, &selected, MODEL, &[1.0, 0.0], 5),
            Err(ProbeError::RevisionCollision)
        ));
        let historical = resolve_sync_revisions(&dag, doc, Some(rev)).unwrap();
        assert!(historical.historical);
    }

    #[test]
    fn no_chunk_revision_must_still_match_dag_content_hash() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let revision = add_dag_revision(&mut dag, doc, vec![], "expected");
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let source_id = doc.to_string();
        register_source(&db, "vault", &source_id).unwrap();
        replace_revision_chunks(
            &mut db, "vault", &source_id, 1, &revision.to_string(),
            &hash("wrong"), MODEL, 2, &[],
        ).unwrap();
        let selection = resolve_sync_revisions(&dag, doc, None).unwrap();
        assert!(matches!(
            search_sync_verified(&db, "vault", 1, &selection, MODEL, &[1.0, 0.0], 10),
            Err(ProbeError::RevisionCollision)
        ));
    }

    #[test]
    fn revoked_grant_during_search_never_returns_hits() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let revision = add_dag_revision(&mut dag, doc, vec![], "document");
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let source_id = doc.to_string();
        register_source(&db, "vault", &source_id).unwrap();
        put(&mut db, "vault", &source_id, 1, &revision.to_string(), "document", &[1.0, 0.0]);
        let selected = resolve_sync_revisions(&dag, doc, None).unwrap();
        let mut checks = 0usize;
        assert!(matches!(
            search_sync_revalidated(&db, "vault", MODEL, &[1.0, 0.0], 10, || {
                checks += 1;
                if checks == 2 {
                    Err(ProbeError::StaleSource)
                } else {
                    Ok(ReadObservation {
                        grant_epoch: 1, owner_epoch: 1, index_generation: 1,
                        selection: selected.clone(),
                    })
                }
            }),
            Err(ProbeError::StaleSource)
        ));
        assert_eq!(checks, 2);
    }

    #[test]
    fn grant_or_source_epoch_change_rejects_results_but_stable_read_passes() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let revision = add_dag_revision(&mut dag, doc, vec![], "document");
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let source_id = doc.to_string();
        register_source(&db, "vault", &source_id).unwrap();
        put(&mut db, "vault", &source_id, 1, &revision.to_string(), "document", &[1.0, 0.0]);
        let selected = resolve_sync_revisions(&dag, doc, None).unwrap();
        let stable = ReadObservation {
            grant_epoch: 5, owner_epoch: 9, index_generation: 1,
            selection: selected.clone(),
        };
        assert_eq!(
            search_sync_revalidated(&db, "vault", MODEL, &[1.0, 0.0], 10,
                || Ok(stable.clone())).unwrap().hits.len(), 1
        );
        let mut calls = 0;
        assert!(matches!(
            search_sync_revalidated(&db, "vault", MODEL, &[1.0, 0.0], 10, || {
                calls += 1;
                let mut observation = stable.clone();
                if calls == 2 {
                    observation.grant_epoch += 1;
                }
                Ok(observation)
            }),
            Err(ProbeError::StaleSource)
        ));
        let mut calls = 0;
        assert!(matches!(
            search_sync_revalidated(&db, "vault", MODEL, &[1.0, 0.0], 10, || {
                calls += 1;
                let mut observation = stable.clone();
                if calls == 2 {
                    observation.owner_epoch += 1;
                }
                Ok(observation)
            }),
            Err(ProbeError::StaleSource)
        ));
    }

    fn plugin_search_fixture() -> (
        crate::plugins::PermissionBroker,
        crate::plugins::PluginInstanceHandle,
        crate::plugins::VaultId,
        crate::plugins::NoteResource,
    ) {
        use crate::plugins::{
            PermissionBroker, PluginManifest, NoteId, NoteResource, VaultId,
        };
        let baseline = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/plugins/fixtures/valid-v1.json"
        ));
        let mut manifest: serde_json::Value = serde_json::from_str(baseline).unwrap();
        manifest["permissions"]["required"] = serde_json::json!(["notes.search"]);
        manifest["permissions"]["optional"] = serde_json::json!([]);
        let manifest = PluginManifest::parse_json(&manifest.to_string()).unwrap();
        let broker = PermissionBroker::new();
        let vault = VaultId::new_for_host();
        let plugin = broker.register_host_package(&manifest, [0x33; 32], vault).unwrap();
        let note = NoteResource::from_host_index(
            vault, NoteId::new_for_host(), "docs/test.md"
        ).unwrap();
        (broker, plugin, vault, note)
    }

    #[test]
    fn plugin_search_denies_without_grant_and_rejects_wrong_scope() {
        use crate::plugins::{Capability, GrantScope, HostContext};
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let (broker, plugin, vault, note) = plugin_search_fixture();
        let doc = DocumentId::new();
        let mut dag = RevisionDag::new();
        let rev = add_dag_revision(&mut dag, doc, vec![], "private");
        let scope = vault.as_uuid().to_string();
        let document_id = doc.to_string();
        register_source(&db, &scope, &document_id).unwrap();
        put(&mut db, &scope, &document_id, 1, &rev.to_string(), "private", &[1.0, 0.0]);
        let context = || HostContext::from_host_state(vault, Some(note.id));
        let selected = resolve_sync_revisions(&dag, doc, None).unwrap();
        let observe = || Ok(ReadObservation {
            grant_epoch: 1, owner_epoch: 1, index_generation: 1,
            selection: selected.clone(),
        });
        assert!(matches!(
            search_sync_with_plugin_grant(&db, &broker, plugin, &note, doc, &scope,
                MODEL, &[1.0, 0.0], 5, context, observe),
            Err(ProbeError::AccessDenied)
        ));
        broker.grant_host_approved(
            plugin, Capability::NotesSearch,
            GrantScope::selected_notes([note.id]).unwrap(),
        ).unwrap();
        let selected = resolve_sync_revisions(&dag, doc, None).unwrap();
        assert!(matches!(
            search_sync_with_plugin_grant(&db, &broker, plugin, &note, doc, "another-vault",
                MODEL, &[1.0, 0.0], 5, context,
                || Ok(ReadObservation {
                    grant_epoch: 1, owner_epoch: 1, index_generation: 1,
                    selection: selected.clone(),
                })),
            Err(ProbeError::AccessDenied)
        ));
        assert_eq!(
            search_sync_with_plugin_grant(&db, &broker, plugin, &note, doc, &scope,
                MODEL, &[1.0, 0.0], 5, context,
                || Ok(ReadObservation {
                    grant_epoch: 1, owner_epoch: 1, index_generation: 1,
                    selection: selected.clone(),
                })).unwrap().hits.len(), 1
        );
        let other_document = DocumentId::new();
        assert!(matches!(
            search_sync_with_plugin_grant(&db, &broker, plugin, &note,
                other_document, &scope, MODEL, &[1.0, 0.0], 5,
                context, || Ok(ReadObservation {
                    grant_epoch: 1, owner_epoch: 1, index_generation: 1,
                    selection: selected.clone(),
                })),
            Err(ProbeError::AccessDenied)
        ));
    }

    #[test]
    fn plugin_current_note_grant_does_not_authorize_history_or_conflict() {
        use crate::plugins::{Capability, GrantScope, HostContext};
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let (broker, plugin, vault, note) = plugin_search_fixture();
        broker.grant_host_approved(
            plugin, Capability::NotesSearch,
            GrantScope::selected_notes([note.id]).unwrap(),
        ).unwrap();
        let doc = DocumentId::new();
        let mut dag = RevisionDag::new();
        let root = add_dag_revision(&mut dag, doc, vec![], "root");
        let left = add_dag_revision(&mut dag, doc, vec![root], "left");
        let right = add_dag_revision(&mut dag, doc, vec![root], "right");
        let scope = vault.as_uuid().to_string();
        register_source(&db, &scope, &doc.to_string()).unwrap();
        put(&mut db, &scope, &doc.to_string(), 1, &root.to_string(), "root", &[1.0, 0.0]);
        put(&mut db, &scope, &doc.to_string(), 1, &left.to_string(), "left", &[1.0, 0.0]);
        put(&mut db, &scope, &doc.to_string(), 1, &right.to_string(), "right", &[1.0, 0.0]);
        let context = || HostContext::from_host_state(vault, Some(note.id));
        let history = resolve_sync_revisions(&dag, doc, Some(root)).unwrap();
        let conflict = resolve_sync_revisions(&dag, doc, None).unwrap();
        assert!(conflict.conflict);
        for selected in [&history, &conflict] {
            assert!(matches!(
                search_sync_with_plugin_grant(
                    &db, &broker, plugin, &note, doc, &scope, MODEL, &[1.0, 0.0], 5,
                    context, || Ok(ReadObservation {
                        grant_epoch: 22, owner_epoch: 1, index_generation: 1,
                        selection: selected.clone(),
                    })
                ),
                Err(ProbeError::AccessDenied)
            ));
        }
        // History is retained; it is just not disclosed through the grant
        // intended for the present, pinned physical note.
        let authorized_history = lookup(
            &db, &scope, &doc.to_string(), 1, history_to_selection(&root.to_string())
        ).unwrap();
        assert_eq!(authorized_history.hits.len(), 1);
    }

    fn history_to_selection(id: &str) -> RevisionSelection<'_> {
        RevisionSelection::Historical(content(id))
    }

    #[test]
    fn plugin_search_revocation_between_scoring_and_release_is_denied() {
        use crate::plugins::{Capability, GrantScope, HostContext};
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let (broker, plugin, vault, note) = plugin_search_fixture();
        broker.grant_host_approved(
            plugin, Capability::NotesSearch,
            GrantScope::selected_notes([note.id]).unwrap(),
        ).unwrap();
        let doc = DocumentId::new();
        let mut dag = RevisionDag::new();
        let rev = add_dag_revision(&mut dag, doc, vec![], "private");
        let scope = vault.as_uuid().to_string();
        register_source(&db, &scope, &doc.to_string()).unwrap();
        put(&mut db, &scope, &doc.to_string(), 1, &rev.to_string(), "private", &[1.0, 0.0]);
        let selection = resolve_sync_revisions(&dag, doc, None).unwrap();
        let mut observations = 0usize;
        let result = search_sync_with_plugin_grant(
            &db, &broker, plugin, &note, doc, &scope, MODEL, &[1.0, 0.0], 5,
            || HostContext::from_host_state(vault, Some(note.id)),
            || {
                observations += 1;
                if observations == 2 {
                    broker.revoke_all_for_host(plugin).unwrap();
                }
                Ok(ReadObservation {
                    grant_epoch: 1, owner_epoch: 1, index_generation: 1,
                    selection: selection.clone(),
                })
            },
        );
        assert!(matches!(result, Err(ProbeError::AccessDenied)));
        assert_eq!(observations, 2);
    }

    #[test]
    fn plugin_search_rejects_changed_live_context_and_wrong_note_grant() {
        use crate::plugins::{Capability, GrantScope, HostContext, NoteId, NoteResource};
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let (broker, plugin, vault, note) = plugin_search_fixture();
        broker.grant_host_approved(
            plugin, Capability::NotesSearch,
            GrantScope::selected_notes([note.id]).unwrap(),
        ).unwrap();
        let doc = DocumentId::new();
        let mut dag = RevisionDag::new();
        let rev = add_dag_revision(&mut dag, doc, vec![], "private");
        let scope = vault.as_uuid().to_string();
        register_source(&db, &scope, &doc.to_string()).unwrap();
        put(&mut db, &scope, &doc.to_string(), 1, &rev.to_string(), "private", &[1.0, 0.0]);
        let selected = resolve_sync_revisions(&dag, doc, None).unwrap();
        let unselected = NoteResource::from_host_index(
            vault, NoteId::new_for_host(), "docs/other.md"
        ).unwrap();
        assert!(matches!(
            search_sync_with_plugin_grant(&db, &broker, plugin, &unselected, doc, &scope,
                MODEL, &[1.0, 0.0], 5,
                || HostContext::from_host_state(vault, Some(note.id)),
                || Ok(ReadObservation {
                    grant_epoch: 1, owner_epoch: 1, index_generation: 1,
                    selection: selected.clone(),
                })),
            Err(ProbeError::AccessDenied)
        ));
        let mut contexts = 0usize;
        let response = search_sync_with_plugin_grant(
            &db, &broker, plugin, &note, doc, &scope, MODEL, &[1.0, 0.0], 5,
            || {
                contexts += 1;
                if contexts >= 4 {
                    HostContext::from_host_state(vault, Some(unselected.id))
                } else {
                    HostContext::from_host_state(vault, Some(note.id))
                }
            },
            || Ok(ReadObservation {
                grant_epoch: 1, owner_epoch: 1, index_generation: 1,
                selection: selected.clone(),
            }),
        );
        assert!(matches!(response, Err(ProbeError::AccessDenied)));
    }

    #[test]
    fn host_note_binding_rejects_wrong_hash_conflict_and_cross_document_path() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        add_dag_revision(&mut dag, doc, vec![], "first");
        assert_eq!(
            bind_current_note_to_sync(&dag, "notes/claim.md", &hash("first"))
                .unwrap().document_id,
            doc.to_string()
        );
        assert!(bind_current_note_to_sync(&dag, "notes/claim.md", &hash("other")).is_err());
        assert!(bind_current_note_to_sync(&dag, "../claim.md", &hash("first")).is_err());
        add_dag_revision(&mut dag, DocumentId::new(), vec![], "first");
        assert!(bind_current_note_to_sync(&dag, "notes/claim.md", &hash("first")).is_err());
    }

    #[test]
    fn host_note_binding_rejects_conflict_and_pending_and_tombstone() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let root = add_dag_revision(&mut dag, doc, vec![], "root");
        add_dag_revision(&mut dag, doc, vec![root], "left");
        add_dag_revision(&mut dag, doc, vec![root], "right");
        assert!(bind_current_note_to_sync(&dag, "notes/claim.md", &hash("left")).is_err());
        let mut clean = RevisionDag::new();
        let doc = DocumentId::new();
        add_dag_revision(&mut clean, doc, vec![], "root");
        let pending = RevisionRecord {
            id: RevisionId::new(), document_id: doc,
            parents: vec![RevisionId::new()],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/claim.md").unwrap(),
                blob_hash: hash("later"),
            },
        };
        clean.ingest(pending).unwrap();
        assert!(bind_current_note_to_sync(&clean, "notes/claim.md", &hash("root")).is_err());
        let mut erased = RevisionDag::new();
        let doc = DocumentId::new();
        let root = add_dag_revision(&mut erased, doc, vec![], "root");
        erased.ingest(RevisionRecord {
            id: RevisionId::new(), document_id: doc, parents: vec![root],
            operation: RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/claim.md").unwrap(),
            },
        }).unwrap();
        assert!(bind_current_note_to_sync(&erased, "notes/claim.md", &hash("root")).is_err());
    }

    #[test]
    fn current_and_history_are_independent_selectors() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let v1 = hash("v1");
        let v2 = hash("v2");
        put(&mut db, "vault", "note", 1, &v1, "old", &[1.0, 0.0]);
        put(&mut db, "vault", "note", 1, &v2, "new", &[0.0, 1.0]);

        let result = lookup(&db, "vault", "note", 1, current(&v2)).unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].revision_id, v2);
        assert!(result.missing.is_empty());

        let result = lookup(&db, "vault", "note", 1, history(&v1)).unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].revision_id, v1);
    }

    fn add_dag_revision(
        dag: &mut RevisionDag,
        document: DocumentId,
        parents: Vec<RevisionId>,
        text: &str,
    ) -> RevisionId {
        let id = RevisionId::new();
        dag.ingest(RevisionRecord {
            id,
            document_id: document,
            parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/claim.md").unwrap(),
                blob_hash: hash(text),
            },
        }).unwrap();
        id
    }

    #[test]
    fn sync_dag_conflict_and_merge_are_never_resolved_by_arrival_order() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let source = doc.to_string();
        let base = add_dag_revision(&mut dag, doc, vec![], "base");
        let left = add_dag_revision(&mut dag, doc, vec![base], "left");
        let right = add_dag_revision(&mut dag, doc, vec![base], "right");

        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", &source).unwrap();
        put(&mut db, "vault", &source, 1, &left.to_string(), "left", &[1.0, 0.0]);
        put(&mut db, "vault", &source, 1, &right.to_string(), "right", &[0.0, 1.0]);

        let ids: Vec<String> = dag.heads(doc).iter().map(ToString::to_string).collect();
        assert_eq!(ids.len(), 2);
        let refs: Vec<_> = ids.iter().map(|id| content(id)).collect();
        let both = lookup(&db, "vault", &source, 1, RevisionSelection::Conflicted(&refs)).unwrap();
        assert_eq!(both.conflicts, vec![source.clone()]);
        assert_eq!(both.hits.len(), 2);
        assert!(both.missing.is_empty());

        let merged = add_dag_revision(&mut dag, doc, vec![left, right], "merged");
        put(&mut db, "vault", &source, 1, &merged.to_string(), "merged", &[1.0, 1.0]);
        let active = lookup(&db, "vault", &source, 1, current(&merged.to_string())).unwrap();
        assert!(active.conflicts.is_empty());
        assert_eq!(active.hits.len(), 1);
        assert_eq!(active.hits[0].revision_id, merged.to_string());
        let old = lookup(&db, "vault", &source, 1, history(&left.to_string())).unwrap();
        assert_eq!(old.hits.len(), 1);
    }

    #[test]
    fn missing_head_never_silently_becomes_the_only_current_version() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let v1 = hash("branch-a");
        let v2 = hash("branch-b");
        put(&mut db, "vault", "note", 1, &v1, "a", &[1.0, 0.0]);

        let branches = [content(&v1), content(&v2)];
        let result = lookup(&db, "vault", "note", 1, RevisionSelection::Conflicted(&branches)).unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.conflicts, vec!["note".to_string()]);
        assert_eq!(result.missing, vec![SourceRevision {
            source_id: "note".to_string(),
            revision_id: v2.clone(),
        }]);
        let empty = lookup(&db, "vault", "note", 1, current(&v2)).unwrap();
        assert!(empty.hits.is_empty());
        assert_eq!(empty.missing.len(), 1);
        assert!(lookup(&db, "vault", "note", 1, RevisionSelection::Conflicted(&[content(&v1)])).is_err());
    }

    #[test]
    fn index_generation_fences_delayed_jobs_without_deleting_history() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let g1 = register_source(&db, "vault", "note").unwrap();
        let v1 = hash("original");
        let v2 = hash("updated");
        put(&mut db, "vault", "note", g1, &v1, "original", &[1.0, 0.0]);
        let g2 = fence_source(&db, "vault", "note", g1).unwrap();
        let h = hash("text");
        assert!(matches!(
            replace_revision_chunks(&mut db, "vault", "note", g1, &v2,
                &hash("updated"), MODEL, 2,
                &[Chunk { locator: "one", text_hash: &h, embedding: &[1.0, 0.0] }]),
            Err(ProbeError::StaleSource)
        ));
        assert!(matches!(
            lookup(&db, "vault", "note", g1, current(&v1)),
            Err(ProbeError::StaleSource)
        ));
        let old = lookup(&db, "vault", "note", g2, history(&v1)).unwrap();
        assert_eq!(old.hits.len(), 1);
        let new = lookup(&db, "vault", "note", g2, current(&v2)).unwrap();
        assert!(new.hits.is_empty());
        assert_eq!(new.missing.len(), 1);
        put(&mut db, "vault", "note", g2, &v2, "updated", &[0.0, 1.0]);
        assert_eq!(lookup(&db, "vault", "note", g2, current(&v2)).unwrap().hits.len(), 1);
    }

    #[test]
    fn tombstone_conflict_is_reported_but_not_used_as_absence_of_history() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let edit = hash("edit");
        let deleted = hash("delete-event");
        put(&mut db, "vault", "note", 1, &edit, "edit", &[1.0, 0.0]);
        let refs = [
            content(&edit),
            RevisionRef { id: &deleted, kind: RevisionKind::Tombstone },
        ];
        let result = lookup(&db, "vault", "note", 1, RevisionSelection::Conflicted(&refs)).unwrap();
        assert_eq!(result.conflicts, vec!["note".to_string()]);
        assert_eq!(result.hits.len(), 1);
        assert!(result.missing.is_empty());
        assert_eq!(result.tombstones.len(), 1);
        assert_eq!(result.tombstones[0].revision_id, deleted);
    }

    #[test]
    fn access_is_scoped_and_revocation_purges_only_local_cache() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        let revision = hash("same");
        register_source(&db, "private", "note").unwrap();
        register_source(&db, "shared", "note").unwrap();
        put(&mut db, "private", "note", 1, &revision, "secret", &[1.0, 0.0]);
        put(&mut db, "shared", "note", 1, &revision, "public", &[1.0, 0.0]);
        let epoch = revoke_source(&mut db, "private", "note", 1).unwrap();
        assert_eq!(epoch, 2);
        assert!(matches!(
            lookup(&db, "private", "note", 1, current(&revision)),
            Err(ProbeError::StaleSource)
        ));
        assert!(register_source(&db, "private", "note").is_err());
        assert_eq!(lookup(&db, "shared", "note", 1, current(&revision)).unwrap().hits.len(), 1);
        let count: i64 = db.query_row(
            "SELECT count(*) FROM semantic_probe_v2_revisions
             WHERE scope_id='private' AND source_id='note'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn cache_eviction_does_not_erase_revision_and_can_rebuild() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let revision = hash("archive");
        put(&mut db, "vault", "note", 1, &revision, "archive", &[1.0, 0.0]);
        evict_cached_revision(&db, "vault", "note", 1, &revision, MODEL).unwrap();
        let unavailable = lookup(&db, "vault", "note", 1, history(&revision)).unwrap();
        assert!(unavailable.hits.is_empty());
        assert_eq!(unavailable.missing.len(), 1);
        put(&mut db, "vault", "note", 1, &revision, "archive", &[1.0, 0.0]);
        assert_eq!(lookup(&db, "vault", "note", 1, history(&revision)).unwrap().hits.len(), 1);
    }

    #[test]
    fn revision_identity_and_model_dimensions_fail_closed() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let revision = hash("r1");
        put(&mut db, "vault", "note", 1, &revision, "same", &[1.0, 0.0]);
        let h = hash("fragment");
        assert!(matches!(
            replace_revision_chunks(&mut db, "vault", "note", 1,
                &revision, &hash("different"), MODEL, 2,
                &[Chunk { locator: "section:1", text_hash: &h, embedding: &[1.0, 0.0] }]),
            Err(ProbeError::RevisionCollision)
        ));
        assert!(replace_revision_chunks(&mut db, "vault", "note", 1,
            &revision, &hash("same"), MODEL, 3,
            &[Chunk { locator: "section:1", text_hash: &h, embedding: &[1.0, 0.0, 0.0] }]).is_err());
        assert!(replace_revision_chunks(&mut db, "vault", "note", 1,
            &revision, &hash("same"), MODEL, 2,
            &[Chunk { locator: "section:1", text_hash: &h, embedding: &[f32::NAN, 0.0] }]).is_err());
        assert_eq!(lookup(&db, "vault", "note", 1, current(&revision)).unwrap().hits.len(), 1);
    }

    #[test]
    fn corrupted_cache_cannot_return_a_wrong_similarity() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let revision = hash("r1");
        put(&mut db, "vault", "note", 1, &revision, "document", &[1.0, 0.0]);
        db.execute(
            "UPDATE semantic_probe_v2_chunks SET embedding=x'000000000000803f'
             WHERE scope_id='vault' AND source_id='note'",
            [],
        ).unwrap();
        assert!(matches!(
            lookup(&db, "vault", "note", 1, current(&revision)),
            Err(ProbeError::CorruptVector)
        ));
    }

    #[test]
    fn persisted_multiversion_cache_reopens_without_duplication() {
        let temporary = tempfile::tempdir().unwrap();
        let filename = temporary.path().join("multiversion-vector-probe.sqlite");
        let v1 = hash("v1");
        let v2 = hash("v2");
        {
            let mut db = Connection::open(&filename).unwrap();
            prepare(&db).unwrap();
            register_source(&db, "vault", "note").unwrap();
            put(&mut db, "vault", "note", 1, &v1, "v1", &[1.0, 0.0]);
            put(&mut db, "vault", "note", 1, &v2, "v2", &[0.0, 1.0]);
            put(&mut db, "vault", "note", 1, &v1, "v1", &[1.0, 0.0]);
        }
        let db = Connection::open(&filename).unwrap();
        prepare(&db).unwrap();
        assert_eq!(lookup(&db, "vault", "note", 1, history(&v1)).unwrap().hits.len(), 1);
        assert_eq!(lookup(&db, "vault", "note", 1, current(&v2)).unwrap().hits.len(), 1);
    }

    #[test]
    fn indexed_empty_revision_is_distinct_from_missing_version() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let empty = hash("empty");
        let absent = hash("absent");
        replace_revision_chunks(&mut db, "vault", "note", 1, &empty,
            &hash("empty text"), MODEL, 2, &[]).unwrap();
        let first = lookup(&db, "vault", "note", 1, current(&empty)).unwrap();
        assert!(first.hits.is_empty());
        assert!(first.missing.is_empty());
        let second = lookup(&db, "vault", "note", 1, current(&absent)).unwrap();
        assert!(second.hits.is_empty());
        assert_eq!(second.missing.len(), 1);
    }

    #[test]
    fn invalid_selection_and_wrong_scopes_fail_closed() {
        let mut db = Connection::open_in_memory().unwrap();
        prepare(&db).unwrap();
        register_source(&db, "vault", "note").unwrap();
        let v1 = hash("v1");
        put(&mut db, "vault", "note", 1, &v1, "v1", &[1.0, 0.0]);
        let same = [content(&v1), content(&v1)];
        assert!(lookup(&db, "vault", "note", 1, RevisionSelection::Conflicted(&same)).is_err());
        assert!(lookup(&db, "other", "note", 1, current(&v1)).is_err());
        let requested = [
            target("note", 1, current(&v1)),
            target("note", 1, history(&v1)),
        ];
        assert!(search(&db, "vault", &requested, MODEL, &[1.0, 0.0], 10).is_err());
        assert!(search(&db, "vault", &[target("note", 1, current(&v1))].as_slice(),
            MODEL, &[0.0, 0.0], 10).is_err());
    }
}
