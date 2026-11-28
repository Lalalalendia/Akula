use super::{Manager, Notice, PathBuf};
use crate::workspace::speech_jobs::{JobRow, PAGE_SIZE};
use aquilum_core::background_jobs::speech;

impl Manager<'_> {
    pub(super) fn publish(&mut self) {
        self.view.tools = self.config.paths();
        self.view.auto_downloads = self.config.auto_downloads;
        self.view.newer = !self.bounds.is_empty();
        self.view.active = self.worker.as_ref().map(|worker| worker.id);
        self.view.active_source = self.worker.as_ref().map(|worker| worker.source.clone());
        self.view.has_queued = self
            .queue
            .oldest_queued(speech::KIND)
            .is_ok_and(|job| job.is_some());
        self.view.caption_job = self.shared.playback.selected().map(|track| track.id());
        match self
            .queue
            .list(speech::KIND, self.bounds.last().copied(), PAGE_SIZE + 1)
        {
            Ok(jobs) => {
                self.view.older = jobs.len() > PAGE_SIZE as usize;
                self.view.jobs = jobs
                    .into_iter()
                    .take(PAGE_SIZE as usize)
                    .map(|job| JobRow {
                        id: job.id,
                        source: PathBuf::from(job.spec.source),
                        state: job.state,
                        committed: job.committed,
                        total: job.total,
                        error: job.error,
                    })
                    .collect();
            }
            Err(error) => self.view.notice = Some(Notice::Error(error.to_string())),
        }
        self.shared.publish(self.view.clone(), self.wake);
    }
}
