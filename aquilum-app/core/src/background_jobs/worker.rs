use super::*;
use rusqlite::Transaction;

pub(super) fn deadline(current: i64, lifetime: Duration) -> Result<i64> {
    let ms = lifetime.as_millis();
    if !(1_000..=3_600_000).contains(&ms) {
        return Err(Error::Invalid("lease duration"));
    }
    current.checked_add(ms as i64).ok_or(Error::Clock)
}

impl Queue {
    pub(super) fn timed_write<T>(&self, write: impl FnOnce(&Connection, i64) -> Result<T>) -> Result<T> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let result = write(&tx, now()?)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn heartbeat(&self, lease: &Lease, lifetime: Duration) -> Result<()> {
        self.timed_write(|conn, current| {
            let until = deadline(current, lifetime)?;
            changed(conn.execute(
                "UPDATE jobs SET lease_until=max(lease_until,?4)
                 WHERE id=?1 AND generation=?2 AND state='running' AND lease_until>?3",
                params![lease.job, lease.generation, current, until],
            )?)
        })
    }

    /// The same deterministic plan can be supplied again on recovery.
    pub fn set_total(&self, lease: &Lease, total: i64) -> Result<()> {
        if !(0..=MAX_SEGMENTS).contains(&total) {
            return Err(Error::Invalid("segment count"));
        }
        self.timed_write(|conn, current| {
            changed(conn.execute(
                "UPDATE jobs SET total=?4
             WHERE id=?1 AND generation=?2 AND state='running' AND lease_until>?3
             AND (total IS NULL OR total=?4) AND committed<=?4",
                params![lease.job, lease.generation, current, total],
            )?)
        })
    }

    pub fn append(&mut self, lease: &Lease, segment: &Segment) -> Result<()> {
        if !(0..MAX_SEGMENTS).contains(&segment.ordinal)
            || segment.start_ms < 0
            || segment.end_ms <= segment.start_ms
            || segment.payload.len() > MAX_PAYLOAD
        {
            return Err(Error::Invalid("segment"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // A second executor may have claimed the lease while we waited for the
        // SQLite writer. Only the current generation with the expected cursor
        // advances the checkpoint.
        changed(tx.execute(
            "UPDATE jobs SET committed=committed+1
             WHERE id=?1 AND generation=?2 AND state='running' AND lease_until>?3
             AND committed=?4 AND total>committed
             AND ?5>=COALESCE((SELECT end_ms FROM segments WHERE job=?1 AND ordinal=?4-1),0)",
            params![
                lease.job,
                lease.generation,
                now()?,
                segment.ordinal,
                segment.start_ms
            ],
        )?)?;
        tx.execute(
            "INSERT INTO segments(job,ordinal,start_ms,end_ms,payload) VALUES (?1,?2,?3,?4,?5)",
            params![
                lease.job,
                segment.ordinal,
                segment.start_ms,
                segment.end_ms,
                segment.payload
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn finish(&self, lease: &Lease) -> Result<()> {
        self.timed_write(|conn, current| changed(conn.execute(
            "UPDATE jobs SET state='completed',lease_until=NULL
             WHERE id=?1 AND generation=?2 AND state='running' AND lease_until>?3 AND committed=total",
            params![lease.job, lease.generation, current],
        )?))
    }

    pub fn fail(&self, lease: &Lease, message: &str) -> Result<()> {
        valid_text("error", message, 8192)?;
        self.timed_write(|conn, current| {
            changed(conn.execute(
                "UPDATE jobs SET state='failed',lease_until=NULL,error=?4
             WHERE id=?1 AND generation=?2 AND state='running' AND lease_until>?3",
                params![lease.job, lease.generation, current, message],
            )?)
        })
    }
}
