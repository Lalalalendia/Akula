use super::{Command, JobId, Queue, Wake};
use aquilum_core::background_jobs::speech::playback::{MediaSource, Transcript};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc::Sender};

#[derive(Default)]
pub(super) struct Bridge {
    track: Mutex<Option<Arc<Attached>>>,
    updated: AtomicBool,
}

impl Bridge {
    pub(super) fn selected(&self) -> Option<Arc<Attached>> {
        self.track
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(super) fn select(&self, track: Option<Arc<Attached>>, wake: &Wake) {
        *self.track.lock().unwrap_or_else(|error| error.into_inner()) = track;
        self.notify(wake);
    }

    pub(super) fn notify(&self, wake: &Wake) {
        self.updated.store(true, Ordering::Release);
        wake();
    }

    pub(super) fn take_update(&self) -> Option<Option<Arc<Attached>>> {
        self.updated
            .swap(false, Ordering::AcqRel)
            .then(|| self.selected())
    }
}

pub(crate) struct Attached {
    transcript: Transcript,
    tx: Sender<Command>,
    state: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    requested: Option<i64>,
    page: Option<CachedPage>,
}

struct CachedPage {
    index: i64,
    /// (start, end, text) in playback milliseconds.
    cues: Vec<(i64, i64, Arc<str>)>,
    committed: i64,
    complete: bool,
}

impl Attached {
    pub(super) fn new(queue: &Queue, id: JobId, tx: Sender<Command>) -> Result<Self, String> {
        Ok(Self {
            transcript: Transcript::verify(queue, id).map_err(|error| error.to_string())?,
            tx,
            state: Mutex::new(Cache::default()),
        })
    }

    pub(crate) fn id(&self) -> JobId {
        self.transcript.id()
    }

    pub(crate) fn text_at(self: &Arc<Self>, media: &MediaSource, seconds: f64) -> Option<Arc<str>> {
        if !seconds.is_finite() || seconds < 0.0 || !self.transcript.matches(media) {
            return None;
        }
        let ms = (seconds * 1000.0).floor() as i64;
        let index = Transcript::page_at(ms);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.requested != Some(index) {
            state.requested = Some(index);
            if state.page.as_ref().is_none_or(|page| page.index != index) {
                let _ = self.tx.send(Command::CaptionPage(Arc::clone(self), index));
            }
        }
        let page = state.page.as_ref().filter(|page| page.index == index)?;
        // A seek can jump in either direction. Prefer the latest active cue
        // when timestamps overlap, falling back to any earlier ongoing cue.
        let end = page.cues.partition_point(|(start, _, _)| *start <= ms);
        page.cues[..end]
            .iter()
            .rev()
            .find(|(_, end, _)| ms < *end)
            .map(|(_, _, text)| Arc::clone(text))
    }

    pub(super) fn load(&self, queue: &Queue, index: i64) -> Result<bool, String> {
        if self.requested() != Some(index) {
            return Ok(false);
        }
        let page = self
            .transcript
            .page(queue, index)
            .map_err(|error| error.to_string())?;
        let cues = page
            .cues
            .into_iter()
            .map(|cue| (cue.start_ms, cue.end_ms, Arc::from(cue.text)))
            .collect();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.requested != Some(index) {
            return Ok(false);
        }
        state.page = Some(CachedPage {
            index,
            cues,
            committed: page.committed,
            complete: page.complete,
        });
        Ok(true)
    }

    pub(super) fn requested(&self) -> Option<i64> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested
    }

    pub(super) fn update_if_advanced(&self, queue: &Queue) -> Result<bool, String> {
        let pending = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state
                .page
                .as_ref()
                .filter(|page| !page.complete && state.requested == Some(page.index))
                .map(|page| (page.index, page.committed))
        };
        let Some((index, committed)) = pending else {
            return Ok(false);
        };
        if queue
            .get(self.id())
            .map_err(|error| error.to_string())?
            .is_some_and(|job| job.committed != committed)
        {
            self.load(queue, index)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests;
