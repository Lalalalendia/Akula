#[cfg(test)]
mod tests;

use super::{JobId, Lease, Queue, Result, worker::deadline};
use rusqlite::{Connection, OptionalExtension, params};
use std::time::Duration;

impl Queue {
    /// Only jobs that are currently queued are eligible for an automatic run.
    pub fn oldest_queued(&self, kind: &str) -> Result<Option<JobId>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM jobs WHERE kind=?1 AND state='queued' ORDER BY id LIMIT 1",
                [kind],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn claim(
        &self,
        kind: &str,
        job: Option<JobId>,
        lifetime: Duration,
    ) -> Result<Option<Lease>> {
        self.timed_write(|conn, current| claim(conn, kind, job, lifetime, current, false))
    }

    pub fn claim_queued(&self, kind: &str, id: JobId, lifetime: Duration) -> Result<Option<Lease>> {
        self.timed_write(|conn, current| claim(conn, kind, Some(id), lifetime, current, true))
    }

    #[cfg(test)]
    pub(super) fn claim_at(
        &self,
        kind: &str,
        job: Option<JobId>,
        lifetime: Duration,
        current: i64,
    ) -> Result<Option<Lease>> {
        claim(&self.connection, kind, job, lifetime, current, false)
    }
}

fn claim(
    conn: &Connection,
    kind: &str,
    job: Option<JobId>,
    lifetime: Duration,
    current: i64,
    queued_only: bool,
) -> Result<Option<Lease>> {
    let until = deadline(current, lifetime)?;
    Ok(conn
        .query_row(
            "UPDATE jobs SET state='running', generation=generation+1, lease_until=?4
             WHERE id=(SELECT id FROM jobs
                 WHERE kind=?1 AND (?2 IS NULL OR id=?2)
                 AND (state='queued' OR (NOT ?5 AND state='running' AND lease_until<=?3))
                 AND (NOT ?5 OR NOT EXISTS (
                     SELECT 1 FROM jobs active
                     WHERE active.kind=?1 AND active.state='running' AND active.lease_until>?3
                 ))
                 ORDER BY id LIMIT 1)
             RETURNING id,generation",
            params![kind, job, current, until, queued_only],
            |row| {
                Ok(Lease {
                    job: row.get(0)?,
                    generation: row.get(1)?,
                })
            },
        )
        .optional()?)
}
