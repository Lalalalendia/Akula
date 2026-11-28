use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{Store, StoreError, StoredOutcome, valid_text};
use crate::TrackKey;

impl Store {
    /// Identity is global within this database; a repeat gets a new ID.
    pub fn begin(
        &mut self,
        account: &str,
        id: &str,
        track: &TrackKey,
        duration_ms: i64,
    ) -> Result<bool, StoreError> {
        valid_text(account, 128)?;
        valid_text(id, 128)?;
        valid_text(track.provider(), 64)?;
        valid_text(track.id(), 512)?;
        if duration_ms <= 0 {
            return Err(StoreError::InvalidInput);
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(String, String, String, i64)> = tx.query_row(
            "SELECT account,provider,track,duration_ms FROM session WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).optional()?;
        if let Some((a,p,t,d)) = old {
            if a != account || p != track.provider() || t != track.id() || d != duration_ms {
                return Err(StoreError::Conflict);
            }
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO session VALUES(?1,?2,?3,?4,?5,NULL)",
            params![id,account,track.provider(),track.id(),duration_ms],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Append one confirmed interval. Identical retries remain safe even
    /// after finalization. Missing, conflicting or out-of-order data fails.
    pub fn checkpoint(
        &mut self,
        account: &str,
        id: &str,
        seq: i64,
        start: i64,
        end: i64,
    ) -> Result<bool, StoreError> {
        valid_text(account, 128)?;
        valid_text(id, 128)?;
        if seq <= 0 || start < 0 || end <= start {
            return Err(StoreError::InvalidInput);
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = owned_outcome(&tx, account, id)?;
        let old: Option<(i64,i64)> = tx.query_row(
            "SELECT start_utc_ms,end_utc_ms FROM segment WHERE session_id=?1 AND seq=?2",
            params![id,seq],
            |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        if let Some(previous) = old {
            if previous != (start,end) {
                return Err(StoreError::Conflict);
            }
            tx.commit()?;
            return Ok(false);
        }
        if outcome.is_some() {
            return Err(StoreError::Conflict);
        }
        tx.execute("INSERT INTO segment VALUES(?1,?2,?3,?4)", params![id,seq,start,end])?;
        tx.commit()?;
        Ok(true)
    }

    /// For recovery the caller must identify a dead session and own its lifecycle.
    /// Interrupted never adds guessed listening time.
    pub fn finish(
        &mut self,
        account: &str,
        id: &str,
        outcome: StoredOutcome,
    ) -> Result<bool, StoreError> {
        valid_text(account, 128)?;
        valid_text(id, 128)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = owned_outcome(&tx, account, id)? {
            if old != outcome.sql() {
                return Err(StoreError::Conflict);
            }
            tx.commit()?;
            return Ok(false);
        }
        tx.execute("UPDATE session SET outcome=?1 WHERE id=?2 AND account=?3",
            params![outcome.sql(),id,account])?;
        tx.commit()?;
        Ok(true)
    }

    /// Logical deletion only. Host must fence all stale events before clearing.
    /// Never silently discard an active session.
    pub fn clear_account(&mut self, account: &str) -> Result<usize, StoreError> {
        valid_text(account, 128)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM session WHERE account=?1 AND outcome IS NULL",
            [account],
            |r| r.get(0),
        )?;
        if active != 0 {
            return Err(StoreError::ActiveSession);
        }
        let count = tx.execute("DELETE FROM session WHERE account=?1", [account])?;
        tx.commit()?;
        Ok(count)
    }
}

fn owned_outcome(
    tx: &Transaction<'_>,
    account: &str,
    id: &str,
) -> Result<Option<String>, StoreError> {
    tx.query_row(
        "SELECT outcome FROM session WHERE id=?1 AND account=?2",
        params![id,account],
        |r| r.get::<_,Option<String>>(0),
    ).optional()?.ok_or(StoreError::UnknownSession)
}
