//! Bounded reads of committed speech against a verified media revision.

use super::{
    fingerprint, local_file, read_cues, same_file, Cue, Error, Queue, Result, KIND, WINDOW_MS,
}; use crate::background_jobs::JobId;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
const PAGE_WINDOWS: i64 = 4;
pub struct MediaSource {
    path: PathBuf,
    metadata: Metadata,
}

impl MediaSource {
    pub fn open(path: &Path) -> Result<Self> {
        let path = local_file(path)?;
        let metadata = fs::metadata(&path)?;
        Ok(Self { path, metadata })
    }
    pub fn matches(&self, other: &Self) -> bool {
        self.path == other.path && same_file(&self.metadata, &other.metadata).unwrap_or(false)
    }
    pub fn is_current(&self) -> bool {
        Self::open(&self.path).is_ok_and(|current| self.matches(&current))
    }
}

pub struct Transcript {
    id: JobId,
    media: MediaSource,
}

pub struct Page {
    pub cues: Vec<Cue>,
    pub committed: i64,
    pub complete: bool,
}

impl Transcript {
    pub fn verify(queue: &Queue, id: JobId) -> Result<Self> {
        Self::verify_checking(queue, id, &mut || Ok(()))
    }
    pub(super) fn verify_checking(queue: &Queue, id: JobId, check: &mut impl FnMut() -> Result<()>) -> Result<Self> {
        let job = queue
            .get(id)?
            .filter(|job| job.spec.kind == KIND && job.committed > 0)
            .ok_or_else(|| Error::Invalid("no committed speech for this job".into()))?;
        let path = local_file(Path::new(&job.spec.source))?;
        let current = fingerprint(&path, check)?;
        if path.as_os_str() != Path::new(&job.spec.source).as_os_str()
            || current.digest != job.spec.revision
        {
            return Err(Error::Invalid(
                "the media has changed since recognition".into(),
            ));
        }
        Ok(Self {
            id,
            media: MediaSource {
                path,
                metadata: current.metadata,
            },
        })
    }
    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn matches(&self, source: &MediaSource) -> bool {
        self.media.matches(source)
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.media.is_current().then_some(&self.media.path)
    }

    pub fn page_at(position_ms: i64) -> i64 {
        position_ms / WINDOW_MS / PAGE_WINDOWS
    }

    pub fn page(&self, queue: &Queue, page: i64) -> Result<Page> {
        if !self.media.is_current() {
            return Err(Error::Invalid(
                "the media has changed since recognition".into(),
            ));
        }
        let job = queue
            .get(self.id)?
            .filter(|job| job.spec.kind == KIND)
            .ok_or_else(|| Error::Invalid("speech job is unavailable".into()))?;
        let first = page
            .checked_mul(PAGE_WINDOWS)
            .filter(|first| *first >= 0)
            .ok_or_else(|| Error::Invalid("caption page is out of range".into()))?;
        let windows = (job.committed - first).clamp(0, PAGE_WINDOWS);
        let mut cues = if windows == 0 {
            Vec::new()
        } else {
            read_cues(queue, self.id, first, windows as u32)?
                .into_iter()
                .flat_map(|(_, cues)| cues)
                .collect()
        };
        cues.sort_by_key(|cue| (cue.start_ms, cue.end_ms));
        let complete = windows == PAGE_WINDOWS;
        Ok(Page {
            cues,
            committed: job.committed,
            complete,
        })
    }
}
