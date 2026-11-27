use super::{Error, KIND, Queue, Result, edits, playback::Transcript};
use crate::background_jobs::{JobId, State};
use crate::text::cancel::Cancel;
use rusqlite::params;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

const MAX_HITS: usize = 12;
const MAX_CANDIDATES: usize = 96;
const MAX_VERIFICATIONS: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target {
    pub job: JobId,
    pub cue: edits::CueId,
    pub revision: i64,
}

#[derive(Debug)]
pub struct Hit {
    pub target: Target,
    pub path: PathBuf,
    pub start_ms: i64,
    pub snippet: String,
}

#[derive(Default)]
pub struct Session {
    verified: HashMap<JobId, Transcript>,
}

pub fn terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2 && term.len() <= 100)
        .take(12)
        .map(str::to_lowercase)
        .collect()
}

impl Session {
    pub fn search(
        &mut self,
        queue: &Queue,
        query: &str,
        vault: Option<&Path>,
        library: Option<&Path>,
        cancel: &Cancel,
    ) -> Result<Vec<Hit>> {
        let terms = terms(query);
        let expression = terms
            .iter()
            .map(|term| format!("\"{term}\"*"))
            .collect::<Vec<_>>()
            .join(" AND ");
        if expression.is_empty() || vault.is_none() && library.is_none() {
            return Ok(Vec::new());
        }
        let scope = |path: Option<&Path>| {
            path.and_then(Path::to_str).map(|value| {
                format!(
                    "{}{}",
                    value.trim_end_matches(std::path::MAIN_SEPARATOR),
                    std::path::MAIN_SEPARATOR
                )
            })
        };
        let mut stmt = queue.connection.prepare(
            "SELECT c.job, c.ordinal, c.cue, c.revision, j.source, c.start_ms,
                    snippet(speech_fts, 0, '', '', ' … ', 24)
             FROM speech_fts JOIN speech_search_cues c ON c.id=speech_fts.rowid JOIN jobs j ON j.id=c.job
             WHERE speech_fts MATCH ?1 AND j.state='completed'
               AND (instr(j.source,?2)=1 OR instr(j.source,?3)=1)
             ORDER BY rank, c.job DESC, c.ordinal, c.cue LIMIT ?4",
        ).map_err(crate::background_jobs::Error::from)?;
        let rows = stmt
            .query_map(
                params![
                    expression,
                    scope(vault),
                    scope(library),
                    MAX_CANDIDATES as i64
                ],
                |row| {
                    Ok(Hit {
                        target: Target {
                            job: row.get(0)?,
                            cue: edits::CueId {
                                window: row.get(1)?,
                                index: row.get(2)?,
                            },
                            revision: row.get(3)?,
                        },
                        path: PathBuf::from(row.get::<_, String>(4)?),
                        start_ms: row.get(5)?,
                        snippet: row.get::<_, String>(6)?.chars().take(360).collect(),
                    })
                },
            )
            .map_err(crate::background_jobs::Error::from)?;
        let candidates = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(crate::background_jobs::Error::from)?;
        let mut rejected = HashSet::new();
        let mut attempts = 0;
        let mut hits = Vec::new();
        for hit in candidates {
            if cancel.is_cancelled() || hits.len() >= MAX_HITS {
                break;
            }
            if rejected.contains(&hit.target.job) {
                continue;
            }
            let eligible = vault.is_some_and(|root| hit.path.starts_with(root))
                || library.is_some_and(|root| hit.path.parent() == Some(root));
            if !eligible || !hit.path.is_file() || std::fs::File::open(&hit.path).is_err() {
                continue;
            }
            if !self
                .verified
                .get(&hit.target.job)
                .is_some_and(|source| source.current_path() == Some(hit.path.as_path()))
            {
                self.verified.remove(&hit.target.job);
                if attempts >= MAX_VERIFICATIONS {
                    continue;
                }
                attempts += 1;
                let verified = Transcript::verify_checking(queue, hit.target.job, &mut || {
                    if cancel.is_cancelled() {
                        Err(Error::Invalid("speech search cancelled".into()))
                    } else {
                        Ok(())
                    }
                });
                match verified {
                    Ok(source) => {
                        if self.verified.len() >= 32 {
                            self.verified.clear();
                        }
                        self.verified.insert(hit.target.job, source);
                    }
                    Err(_) => {
                        rejected.insert(hit.target.job);
                        continue;
                    }
                }
            }
            hits.push(hit);
        }
        Ok(hits)
    }
}

pub fn locate(queue: &Queue, target: Target) -> Result<(edits::Page, Transcript, i64)> {
    if !queue
        .get(target.job)?
        .is_some_and(|job| job.spec.kind == KIND && job.state == State::Completed)
    {
        return Err(Error::Invalid(
            "the speech search result is no longer available".into(),
        ));
    }
    let media = Transcript::verify(queue, target.job)?;
    let page = edits::page(queue, target.job, edits::page_of(target.cue))?;
    let row = page
        .rows
        .iter()
        .find(|row| {
            row.id == target.cue && row.revision == target.revision && !row.cue.text.is_empty()
        })
        .ok_or_else(|| Error::Invalid("the speech search result changed. Search again".into()))?;
    let time = row.cue.start_ms;
    Ok((page, media, time))
}

#[cfg(test)]
mod tests;
