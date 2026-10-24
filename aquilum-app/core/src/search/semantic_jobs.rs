//! An opt-in, durable scheduling experiment for semantic vector indexing.
//!
//! This does NOT spawn a daemon, load a model or grant file access. The trusted
//! host supplies a verified, immutable source revision and versioned pipeline
//! key (model + extractor + chunker). The queue stores metadata only, never
//! document text, vectors, secrets or file paths. Only a separately admitted
//! worker may read source bytes and call semantic_probe::replace_revision_chunks.
//!
//! The SQLite lease is an at-least-once compute mechanism, NOT write authority:
//! before inserting vectors the consumer must revalidate the source generation,
//! blob hash and its live read capability. A late worker may waste computation,
//! but cannot acknowledge a stale lease. A separate, host-owned reader enforces
//! cancellation and permissions at the read and result delivery boundaries.
//!
//! Important: semantic_probe::prepare must be called before prepare_jobs. Keep
//! this alongside the experiment; do not add it to the existing Tantivy worker
//! or expose it to MCP/UI until real embedding and host admission exist.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::fmt;

const MAX_JOBS_PER_SCOPE: i64 = 20_000;
const MAX_ATTEMPTS: i64 = 5;
const MAX_LEASE_SECONDS: i64 = 600;

#[derive(Debug)]
pub(crate) enum QueueError {
    Invalid(&'static str),
    StaleSource,
    RevisionCollision,
    AtCapacity,
    LeaseLost,
    NotIndexed,
    Sql(rusqlite::Error),
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(msg) => f.write_str(msg),
            Self::StaleSource => f.write_str("Source registration or generation changed"),
            Self::RevisionCollision => f.write_str("Immutable revision was reused with a different hash"),
            Self::AtCapacity => f.write_str("Semantic job queue reached its bounded capacity"),
            Self::LeaseLost => f.write_str("Vector job lease was lost or expired"),
            Self::NotIndexed => f.write_str("Selected immutable vector revision is not indexed"),
            Self::Sql(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for QueueError {}

impl From<rusqlite::Error> for QueueError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sql(err)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JobKey {
    pub scope_id: String,
    pub source_id: String,
    pub revision_id: String,
    /// Must encode model, weights revision, extractor and chunker revisions.
    /// Changing any of them produces a DIFFERENT indexed model identity.
    pub pipeline_id: String,
    pub source_hash: String,
}

impl JobKey {
    fn validate(&self) -> Result<(), QueueError> {
        for item in [&self.scope_id, &self.source_id, &self.revision_id, &self.pipeline_id] {
            if item.is_empty() || item.len() > 512 || item.chars().any(char::is_control) {
                return Err(QueueError::Invalid("Bad source or pipeline identity"));
            }
        }
        if self.source_hash.len() != 64 || !self.source_hash.bytes().all(|b| {
            b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
        }) {
            return Err(QueueError::Invalid("Expected lowercase BLAKE3 source hash"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Lease {
    pub key: JobKey,
    pub local_generation: u64,
    pub lease_token: u64,
    pub lease_until: i64,
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct QueueStats {
    pub pending: u64,
    pub running: u64,
    pub complete: u64,
    pub failed: u64,
}

/// Initialize a separate, disposable queue. The ON UPDATE trigger means that
/// semantic_probe::revoke_source purges all queued metadata for that source
/// while retaining the revoked source tombstone (which blocks re-enrollment).
pub(crate) fn prepare_jobs(connection: &Connection) -> Result<(), QueueError> {
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS semantic_probe_v2_jobs (
             scope_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             revision_id TEXT NOT NULL,
             pipeline_id TEXT NOT NULL,
             source_hash TEXT NOT NULL,
             local_generation INTEGER NOT NULL CHECK(local_generation > 0),
             priority INTEGER NOT NULL CHECK(priority BETWEEN 0 AND 3),
             state TEXT NOT NULL CHECK(state IN ('pending','running','complete','failed')),
             attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
             lease_token INTEGER NOT NULL DEFAULT 0 CHECK(lease_token >= 0),
             lease_until INTEGER,
             PRIMARY KEY(scope_id,source_id,revision_id,pipeline_id),
             FOREIGN KEY(scope_id,source_id)
               REFERENCES semantic_probe_v2_sources(scope_id,source_id) ON DELETE CASCADE
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS semantic_probe_v2_job_order
           ON semantic_probe_v2_jobs(scope_id,state,priority DESC,attempts);
         CREATE TRIGGER IF NOT EXISTS semantic_probe_v2_revoke_jobs
         AFTER UPDATE OF revoked ON semantic_probe_v2_sources
         WHEN new.revoked=1
         BEGIN
             DELETE FROM semantic_probe_v2_jobs
             WHERE scope_id=new.scope_id AND source_id=new.source_id;
         END;",
    )?;
    Ok(())
}

fn check_generation(
    transaction: &rusqlite::Transaction<'_>,
    key: &JobKey,
    generation: u64,
) -> Result<i64, QueueError> {
    if generation == 0 || generation >= i64::MAX as u64 {
        return Err(QueueError::Invalid("Invalid local generation"));
    }
    let got: Option<i64> = transaction.query_row(
        "SELECT generation FROM semantic_probe_v2_sources
         WHERE scope_id=?1 AND source_id=?2 AND revoked=0",
        params![key.scope_id, key.source_id],
        |row| row.get(0),
    ).optional()?;
    if got != Some(generation as i64) {
        return Err(QueueError::StaleSource);
    }
    Ok(generation as i64)
}

/// Enqueue a verified immutable revision, or coalesce an identical event.
/// A change to local generation requeues the same immutable revision with a
/// fresh lease fence. A genuine collision never overwrites the old identity.
/// Priority: 3 current visible, 2 visible conflict, 1 historical, 0 speculative.
pub(crate) fn enqueue(
    connection: &mut Connection,
    key: &JobKey,
    generation: u64,
    priority: u8,
) -> Result<(), QueueError> {
    key.validate()?;
    if priority > 3 {
        return Err(QueueError::Invalid("Invalid queue priority"));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let generation = check_generation(&transaction, key, generation)?;
    let existing: Option<(String, i64, String)> = transaction.query_row(
        "SELECT source_hash,local_generation,state FROM semantic_probe_v2_jobs
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND pipeline_id=?4",
        params![key.scope_id, key.source_id, key.revision_id, key.pipeline_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;

    match existing {
        Some((hash, previous_generation, state)) => {
            if hash != key.source_hash {
                return Err(QueueError::RevisionCollision);
            }
            if previous_generation != generation {
                transaction.execute(
                    "UPDATE semantic_probe_v2_jobs
                     SET local_generation=?5,priority=?6,state='pending',
                         attempts=0,lease_token=lease_token+1,lease_until=NULL
                     WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND pipeline_id=?4",
                    params![key.scope_id,key.source_id,key.revision_id,key.pipeline_id,
                            generation,i64::from(priority)],
                )?;
            } else {
                // Do not restart a completed or exhausted job just because a
                // watcher sent the same event again. Requeue after explicit
                // cache eviction or pipeline revision using a separate action.
                let _ = state;
                transaction.execute(
                    "UPDATE semantic_probe_v2_jobs SET priority=MAX(priority,?5)
                     WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND pipeline_id=?4",
                    params![key.scope_id,key.source_id,key.revision_id,key.pipeline_id,
                            i64::from(priority)],
                )?;
            }
        }
        None => {
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM semantic_probe_v2_jobs WHERE scope_id=?1",
                params![key.scope_id],
                |row| row.get(0),
            )?;
            if count >= MAX_JOBS_PER_SCOPE {
                return Err(QueueError::AtCapacity);
            }
            transaction.execute(
                "INSERT INTO semantic_probe_v2_jobs
                    (scope_id,source_id,revision_id,pipeline_id,source_hash,
                     local_generation,priority,state)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,'pending')",
                params![key.scope_id,key.source_id,key.revision_id,key.pipeline_id,
                        key.source_hash,generation,i64::from(priority)],
            )?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// Claim one item in a serialized SQLite write transaction. Expired leases
/// become reclaimable after a process crash; actual work is at least once,
/// with a bounded retry count and a monotonically changing lease token.
pub(crate) fn claim_next(
    connection: &mut Connection,
    scope_id: &str,
    now: i64,
    lease_seconds: i64,
) -> Result<Option<Lease>, QueueError> {
    if scope_id.is_empty() || scope_id.len() > 512 || now < 0
        || !(1..=MAX_LEASE_SECONDS).contains(&lease_seconds)
    {
        return Err(QueueError::Invalid("Invalid queue scope or clock/lease"));
    }
    let deadline = now.checked_add(lease_seconds)
        .ok_or(QueueError::Invalid("Lease clock overflow"))?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let job: Option<(String,String,String,String,i64,i64,i64)> = transaction.query_row(
        "SELECT j.source_id,j.revision_id,j.pipeline_id,j.source_hash,
                j.local_generation,j.lease_token,j.attempts
         FROM semantic_probe_v2_jobs j
         JOIN semantic_probe_v2_sources s
           ON s.scope_id=j.scope_id AND s.source_id=j.source_id
          AND s.generation=j.local_generation AND s.revoked=0
         WHERE j.scope_id=?1 AND j.attempts<?3
           AND (j.state='pending'
             OR (j.state='running' AND j.lease_until<=?2))
         ORDER BY j.priority DESC,j.attempts ASC,j.source_id,j.revision_id
         LIMIT 1",
        params![scope_id,now,MAX_ATTEMPTS],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,
                  row.get(4)?,row.get(5)?,row.get(6)?)),
    ).optional()?;
    let Some((source_id,revision_id,pipeline_id,source_hash,generation,token,attempt))=job else {
        transaction.commit()?;
        return Ok(None);
    };
    if token >= i64::MAX-1 || attempt >= MAX_ATTEMPTS {
        return Err(QueueError::Invalid("Exhausted lease token"));
    }
    let token=token+1;
    transaction.execute(
        "UPDATE semantic_probe_v2_jobs
         SET state='running',lease_token=?5,lease_until=?6,attempts=attempts+1
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND pipeline_id=?4",
        params![scope_id,source_id,revision_id,pipeline_id,token,deadline],
    )?;
    transaction.commit()?;
    Ok(Some(Lease {
        key: JobKey {
            scope_id: scope_id.to_owned(),
            source_id, revision_id, pipeline_id, source_hash,
        },
        local_generation: generation as u64,
        lease_token: token as u64,
        lease_until: deadline,
        attempt: (attempt+1) as u32,
    }))
}

/// Extend one live lease during long-running CPU/GPU inference. Caller uses
/// real host clock and cannot revive a superseded token or revoked source.
pub(crate) fn renew(
    connection: &Connection,
    lease: &Lease,
    now: i64,
    lease_seconds: i64,
) -> Result<i64, QueueError> {
    if now < 0 || !(1..=MAX_LEASE_SECONDS).contains(&lease_seconds) {
        return Err(QueueError::Invalid("Invalid renewal clock"));
    }
    let deadline=now.checked_add(lease_seconds)
        .ok_or(QueueError::Invalid("Lease clock overflow"))?;
    let changed=connection.execute(
        "UPDATE semantic_probe_v2_jobs SET lease_until=?8
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3
           AND pipeline_id=?4 AND lease_token=?5 AND state='running'
           AND local_generation=?6 AND lease_until>?7
           AND EXISTS(SELECT 1 FROM semantic_probe_v2_sources s
             WHERE s.scope_id=?1 AND s.source_id=?2
               AND s.generation=?6 AND s.revoked=0)",
        params![lease.key.scope_id,lease.key.source_id,lease.key.revision_id,
                lease.key.pipeline_id,lease.lease_token as i64,
                lease.local_generation as i64,now,deadline],
    )?;
    if changed != 1 {
        return Err(QueueError::LeaseLost);
    }
    Ok(deadline)
}

/// Acknowledge only if a separate trusted worker has already written the
/// exact immutable source hash into the vector cache for this pipeline.
/// This does not authorize a source read, a text snippet, or an MCP result.
pub(crate) fn acknowledge_indexed(
    connection: &mut Connection,
    lease: &Lease,
    now: i64,
) -> Result<(), QueueError> {
    if now < 0 {
        return Err(QueueError::Invalid("Invalid completion clock"));
    }
    let transaction=connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_lease(&transaction,lease,now)?;
    let ready: Option<i64> = transaction.query_row(
        "SELECT 1 FROM semantic_probe_v2_ready r
         JOIN semantic_probe_v2_revisions v
          ON v.scope_id=r.scope_id AND v.source_id=r.source_id
         AND v.revision_id=r.revision_id
         WHERE r.scope_id=?1 AND r.source_id=?2 AND r.revision_id=?3
          AND r.model_id=?4 AND v.source_hash=?5",
        params![lease.key.scope_id,lease.key.source_id,lease.key.revision_id,
                lease.key.pipeline_id,lease.key.source_hash],
        |row| row.get(0),
    ).optional()?;
    if ready.is_none() {
        return Err(QueueError::NotIndexed);
    }
    transaction.execute(
        "UPDATE semantic_probe_v2_jobs SET state='complete',lease_until=NULL
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND pipeline_id=?4",
        params![lease.key.scope_id,lease.key.source_id,lease.key.revision_id,
                lease.key.pipeline_id],
    )?;
    transaction.commit()?;
    Ok(())
}

fn ensure_lease(
    transaction: &rusqlite::Transaction<'_>,
    lease: &Lease,
    now: i64,
) -> Result<(), QueueError> {
    let current: Option<i64> = transaction.query_row(
        "SELECT 1 FROM semantic_probe_v2_jobs j
         JOIN semantic_probe_v2_sources s
          ON s.scope_id=j.scope_id AND s.source_id=j.source_id
          AND s.generation=j.local_generation AND s.revoked=0
         WHERE j.scope_id=?1 AND j.source_id=?2 AND j.revision_id=?3
           AND j.pipeline_id=?4 AND j.source_hash=?5
           AND j.local_generation=?6 AND j.lease_token=?7
           AND j.state='running' AND j.lease_until>?8",
        params![lease.key.scope_id,lease.key.source_id,lease.key.revision_id,
                lease.key.pipeline_id,lease.key.source_hash,
                lease.local_generation as i64,lease.lease_token as i64,now],
        |row| row.get(0),
    ).optional()?;
    if current.is_none() {
        return Err(QueueError::LeaseLost);
    }
    Ok(())
}

/// Recoverable failures retry up to MAX_ATTEMPTS; further errors remain as
/// failed evidence rather than creating a spin loop. Repeated watcher events
/// cannot silently reset exhausted attempts for the same source generation.
pub(crate) fn fail(
    connection: &mut Connection,
    lease: &Lease,
    now: i64,
) -> Result<(), QueueError> {
    if now < 0 {
        return Err(QueueError::Invalid("Invalid failure clock"));
    }
    let transaction=connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_lease(&transaction,lease,now)?;
    transaction.execute(
        "UPDATE semantic_probe_v2_jobs
         SET state=CASE WHEN attempts>=?5 THEN 'failed' ELSE 'pending' END,
             lease_until=NULL
         WHERE scope_id=?1 AND source_id=?2 AND revision_id=?3 AND pipeline_id=?4",
        params![lease.key.scope_id,lease.key.source_id,lease.key.revision_id,
                lease.key.pipeline_id,MAX_ATTEMPTS],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn stats(connection: &Connection, scope_id: &str) -> Result<QueueStats, QueueError> {
    if scope_id.is_empty() || scope_id.len()>512 {
        return Err(QueueError::Invalid("Bad queue scope"));
    }
    let mut stmt=connection.prepare(
        "SELECT state,COUNT(*) FROM semantic_probe_v2_jobs
         WHERE scope_id=?1 GROUP BY state",
    )?;
    let mut rows=stmt.query(params![scope_id])?;
    let mut count=QueueStats::default();
    while let Some(row)=rows.next()? {
        let state: String=row.get(0)?;
        let value: u64=row.get(1)?;
        match state.as_str() {
            "pending"=>count.pending=value,
            "running"=>count.running=value,
            "complete"=>count.complete=value,
            "failed"=>count.failed=value,
            _=>return Err(QueueError::Invalid("Unknown job state")),
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::semantic_probe::{self, Chunk};

    fn revision(text: &str) -> String {
        blake3::hash(text.as_bytes()).to_hex().to_string()
    }

    fn db() -> Connection {
        let connection=Connection::open_in_memory().unwrap();
        semantic_probe::prepare(&connection).unwrap();
        prepare_jobs(&connection).unwrap();
        connection
    }

    fn source(connection: &Connection, scope: &str, src: &str) {
        semantic_probe::register_source(connection,scope,src).unwrap();
    }

    fn key(scope: &str,source: &str,rev: &str) -> JobKey {
        JobKey {
            scope_id:scope.into(),source_id:source.into(),
            revision_id:rev.into(),pipeline_id:"embed-fixture@1+markdown-v1".into(),
            source_hash:revision("original source"),
        }
    }

    #[test]
    fn coalescing_does_not_duplicate_or_repeat_completed_work() {
        let mut c=db();
        source(&c,"vault","note");
        let item=key("vault","note",&revision("v1"));
        enqueue(&mut c,&item,1,1).unwrap();
        enqueue(&mut c,&item,1,3).unwrap();
        assert_eq!(stats(&c,"vault").unwrap().pending,1);
        let lease=claim_next(&mut c,"vault",100,60).unwrap().unwrap();
        assert_eq!(lease.attempt,1);
        assert_eq!(lease.key,item);
        let fragment=revision("fragment");
        semantic_probe::replace_revision_chunks(
            &mut c,"vault","note",1,&item.revision_id,&item.source_hash,
            &item.pipeline_id,2,
            &[Chunk{locator:"section1",text_hash:&fragment,embedding:&[1.0,0.0]}],
        ).unwrap();
        acknowledge_indexed(&mut c,&lease,110).unwrap();
        enqueue(&mut c,&item,1,1).unwrap();
        assert!(claim_next(&mut c,"vault",111,60).unwrap().is_none());
        assert_eq!(stats(&c,"vault").unwrap().complete,1);
    }

    #[test]
    fn restart_reclaims_expired_lease_but_never_acks_stale_token() {
        let temp=tempfile::tempdir().unwrap();
        let path=temp.path().join("jobs.sqlite");
        let item=key("vault","note",&revision("v1"));
        let old;
        {
            let mut c=Connection::open(&path).unwrap();
            semantic_probe::prepare(&c).unwrap();
            prepare_jobs(&c).unwrap();
            source(&c,"vault","note");
            enqueue(&mut c,&item,1,2).unwrap();
            old=claim_next(&mut c,"vault",100,10).unwrap().unwrap();
        }
        let mut c=Connection::open(&path).unwrap();
        semantic_probe::prepare(&c).unwrap();
        prepare_jobs(&c).unwrap();
        assert!(claim_next(&mut c,"vault",109,10).unwrap().is_none());
        let new=claim_next(&mut c,"vault",111,10).unwrap().unwrap();
        assert!(new.lease_token>old.lease_token);
        assert_eq!(new.attempt,2);
        assert!(matches!(fail(&mut c,&old,112),Err(QueueError::LeaseLost)));
        assert!(matches!(acknowledge_indexed(&mut c,&old,112),Err(QueueError::LeaseLost)));
        renew(&c,&new,115,20).unwrap();
        fail(&mut c,&new,116).unwrap();
        assert_eq!(stats(&c,"vault").unwrap().pending,1);
    }

    #[test]
    fn source_fence_invalidates_old_worker_and_requeues_current_generation() {
        let mut c=db();
        source(&c,"vault","note");
        let item=key("vault","note",&revision("v1"));
        enqueue(&mut c,&item,1,3).unwrap();
        let old=claim_next(&mut c,"vault",100,20).unwrap().unwrap();
        assert_eq!(semantic_probe::fence_source(&c,"vault","note",1).unwrap(),2);
        assert!(matches!(enqueue(&mut c,&item,1,1),Err(QueueError::StaleSource)));
        enqueue(&mut c,&item,2,3).unwrap();
        assert!(matches!(acknowledge_indexed(&mut c,&old,103),Err(QueueError::LeaseLost)));
        let new=claim_next(&mut c,"vault",104,20).unwrap().unwrap();
        assert_eq!(new.local_generation,2);
        assert_ne!(old.lease_token,new.lease_token);
    }

    #[test]
    fn immutable_revision_collision_rejected_and_no_cross_vault_claim() {
        let mut c=db();
        source(&c,"private","note");
        source(&c,"shared","note");
        let item=key("private","note",&revision("r1"));
        enqueue(&mut c,&item,1,2).unwrap();
        let mut changed=item.clone();
        changed.source_hash=revision("other bytes");
        assert!(matches!(enqueue(&mut c,&changed,1,2),Err(QueueError::RevisionCollision)));
        assert!(claim_next(&mut c,"shared",100,20).unwrap().is_none());
        assert_eq!(claim_next(&mut c,"private",100,20).unwrap().unwrap().key,item);
    }

    #[test]
    fn source_revocation_removes_all_queued_metadata_and_blocks_reenroll() {
        let mut c=db();
        source(&c,"vault","note");
        let item=key("vault","note",&revision("r1"));
        enqueue(&mut c,&item,1,0).unwrap();
        let worker=claim_next(&mut c,"vault",100,30).unwrap().unwrap();
        semantic_probe::revoke_source(&mut c,"vault","note",1).unwrap();
        assert_eq!(stats(&c,"vault").unwrap(),QueueStats::default());
        assert!(matches!(acknowledge_indexed(&mut c,&worker,110),Err(QueueError::LeaseLost)));
        assert!(matches!(enqueue(&mut c,&item,1,1),Err(QueueError::StaleSource)));
        assert!(semantic_probe::register_source(&c,"vault","note").is_err());
    }

    #[test]
    fn ack_requires_matching_immutable_cache_not_just_a_finished_model() {
        let mut c=db();
        source(&c,"vault","note");
        let item=key("vault","note",&revision("r1"));
        enqueue(&mut c,&item,1,2).unwrap();
        let leased=claim_next(&mut c,"vault",100,20).unwrap().unwrap();
        assert!(matches!(acknowledge_indexed(&mut c,&leased,101),Err(QueueError::NotIndexed)));
        let frag=revision("frag");
        semantic_probe::replace_revision_chunks(
            &mut c,"vault","note",1,&item.revision_id,&revision("wrong bytes"),
            &item.pipeline_id,2,
            &[Chunk{locator:"p",text_hash:&frag,embedding:&[1.0,0.0]}],
        ).unwrap();
        assert!(matches!(acknowledge_indexed(&mut c,&leased,101),Err(QueueError::NotIndexed)));
        // A bad immutable identity cannot be overwritten by a different hash;
        // the worker must fail and reconcile with the source authority.
        assert!(semantic_probe::replace_revision_chunks(
            &mut c,"vault","note",1,&item.revision_id,&item.source_hash,
            &item.pipeline_id,2,&[],
        ).is_err());
    }

    #[test]
    fn repeated_failures_stay_failed_instead_of_spinning() {
        let mut c=db();
        source(&c,"vault","note");
        let item=key("vault","note",&revision("r1"));
        enqueue(&mut c,&item,1,1).unwrap();
        for n in 0..MAX_ATTEMPTS {
            let lease=claim_next(&mut c,"vault",100+n*2,5).unwrap().unwrap();
            fail(&mut c,&lease,101+n*2).unwrap();
        }
        assert!(claim_next(&mut c,"vault",200,5).unwrap().is_none());
        assert_eq!(stats(&c,"vault").unwrap().failed,1);
        enqueue(&mut c,&item,1,3).unwrap();
        assert_eq!(stats(&c,"vault").unwrap().failed,1);
    }
}
