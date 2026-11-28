//! User corrections for committed speech. Window payloads remain append-only.

use super::{Cue, Error, JobId, KIND, Queue, Result};
use rusqlite::{Transaction, TransactionBehavior, params};
const WINDOWS_PER_PAGE: i64 = 4;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CueId {
    pub window: i64,
    pub index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub id: CueId,
    pub cue: Cue,
    pub revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    pub source: String,
    pub number: i64,
    pub pages: i64,
    pub rows: Vec<Row>,
}

pub fn page(queue: &Queue, id: JobId, number: i64) -> Result<Page> {
    let _snapshot = snapshot(queue)?;
    let job = queue
        .get(id)?
        .filter(|job| job.spec.kind == KIND)
        .ok_or_else(|| Error::Invalid("speech job is unavailable".into()))?;
    let pages = ((job.committed + WINDOWS_PER_PAGE - 1) / WINDOWS_PER_PAGE).max(1);
    let number = number.clamp(0, pages - 1);
    let rows = windows(
        queue,
        id,
        number * WINDOWS_PER_PAGE,
        WINDOWS_PER_PAGE as u32,
    )?
    .into_iter()
    .flat_map(|(_, rows)| rows)
    .collect();
    Ok(Page {
        source: job.spec.source,
        number,
        pages,
        rows,
    })
}

pub fn save(queue: &Queue, job: JobId, id: CueId, expected: i64, text: &str) -> Result<Row> {
    if text.len() > super::super::MAX_PAYLOAD || text.contains('\0') || expected < 0 {
        return Err(Error::Invalid("invalid caption text or revision".into()));
    }
    let tx = Transaction::new_unchecked(&queue.connection, TransactionBehavior::Immediate)
        .map_err(super::super::Error::from)?;
    if !queue
        .get(job)?
        .is_some_and(|job| job.spec.kind == KIND && (0..job.committed).contains(&id.window))
    {
        return Err(Error::Invalid("speech window is unavailable".into()));
    }
    let mut row = windows(queue, job, id.window, 1)?
        .into_iter()
        .next()
        .filter(|(window, _)| *window == id.window)
        .and_then(|(_, rows)| rows.into_iter().nth(id.index as usize))
        .ok_or_else(|| Error::Invalid("speech cue is unavailable".into()))?;
    if row.revision != expected {
        return Err(Error::Invalid(
            "this cue was edited elsewhere. Reload before saving".into(),
        ));
    }
    let text = text.trim();
    if row.cue.text != text {
        let revision = expected
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("caption revision overflow".into()))?;
        tx.execute(
            "INSERT INTO speech_cue_edits(job, ordinal, cue, revision, text) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(job,ordinal,cue) DO UPDATE SET revision=excluded.revision,text=excluded.text",
            params![job, id.window, id.index, revision, text],
        ).map_err(super::super::Error::from)?;
        row.revision = revision;
        row.cue.text = text.to_owned();
    }
    tx.commit().map_err(super::super::Error::from)?;
    Ok(row)
}

pub fn page_of(cue: CueId) -> i64 { cue.window / WINDOWS_PER_PAGE }

pub(super) fn snapshot(queue: &Queue) -> Result<Transaction<'_>> {
    Ok(queue
        .connection
        .unchecked_transaction()
        .map_err(super::super::Error::from)?)
}

/// Caller owns the read or write transaction spanning windows and edits.
pub(super) fn windows(
    queue: &Queue,
    job: JobId,
    first: i64,
    count: u32,
) -> Result<Vec<(i64, Vec<Row>)>> {
    let segments = queue.segments(job, first, count)?;
    let Some(last) = segments.last() else {
        return Ok(Vec::new());
    };
    let mut stmt = queue
        .connection
        .prepare(
            "SELECT ordinal,cue,revision,text FROM speech_cue_edits
         WHERE job=?1 AND ordinal BETWEEN ?2 AND ?3 ORDER BY ordinal,cue",
        )
        .map_err(super::super::Error::from)?;
    let records = stmt
        .query_map(params![job, first, last.ordinal], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(super::super::Error::from)?;
    let edits = records
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(super::super::Error::from)?;
    let mut edits = edits.into_iter().peekable();
    let mut output = Vec::with_capacity(segments.len());
    for segment in segments {
        let list: Vec<Cue> = serde_json::from_slice(&segment.payload)?;
        let mut rows: Vec<Row> = list
            .into_iter()
            .enumerate()
            .map(|(index, cue)| Row {
                id: CueId {
                    window: segment.ordinal,
                    index: index as u32,
                },
                cue,
                revision: 0,
            })
            .collect();
        while edits.peek().is_some_and(|edit| edit.0 == segment.ordinal) {
            let (_, index, revision, text) = edits.next().expect("matching edit");
            let row = rows
                .get_mut(index as usize)
                .ok_or_else(|| Error::Invalid("stored edit has no speech cue".into()))?;
            row.revision = revision;
            row.cue.text = text;
        }
        output.push((segment.ordinal, rows));
    }
    Ok(output)
}

pub(super) fn effective(rows: Vec<Row>) -> impl Iterator<Item = Cue> {
    rows.into_iter()
        .map(|row| row.cue)
        .filter(|cue| !cue.text.is_empty())
}

/// Read a page of committed windows with their timestamped cues.
pub fn read_cues(queue: &Queue, id: JobId, first: i64, count: u32) -> Result<Vec<(i64, Vec<Cue>)>> {
    let _snapshot = snapshot(queue)?;
    if !queue.get(id)?.is_some_and(|job| job.spec.kind == KIND) {
        return Err(Error::Invalid(format!("speech job {id} does not exist")));
    }
    Ok(windows(queue, id, first, count)?
        .into_iter()
        .map(|(window, rows)| (window, effective(rows).collect()))
        .collect())
}

#[cfg(test)]
mod tests;
