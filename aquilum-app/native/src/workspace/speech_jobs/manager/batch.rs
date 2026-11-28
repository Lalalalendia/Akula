use super::{Manager, Notice, PathBuf};
use crate::workspace::speech_jobs::BatchReport;
use aquilum_core::background_jobs::{Error, JobId, speech};

impl Manager<'_> {
    pub(super) fn add_batch(&mut self, sources: Vec<PathBuf>) -> Result<(), String> {
        if sources.len() > speech::MAX_BATCH_FILES {
            self.view.notice = Some(Notice::BatchTooLarge);
            return Ok(());
        }
        self.view.selection = None;
        let tools = self.config.verify()?;
        let results = speech::enqueue_batch(&self.queue, &tools, &sources, "auto")
            .map_err(|error| error.to_string())?;
        let mut report = BatchReport::default();
        for (path, result) in sources.iter().zip(results) {
            match result {
                Ok(_) => report.accepted += 1,
                Err(error) => {
                    report.rejected += 1;
                    report.example.get_or_insert_with(|| {
                        let name = path
                            .file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy();
                        format!("{name} — {error}").chars().take(200).collect()
                    });
                }
            }
        }
        if report.accepted > 0 {
            self.bounds.clear();
        }
        self.view.notice = Some(Notice::Batch(report));
        Ok(())
    }

    pub(super) fn run_queue(&mut self) -> Result<(), String> {
        if !self.view.ready() {
            return Err("set up the speech tools and finish the active job first".into());
        }
        self.view.processing_queue = true;
        Ok(())
    }

    pub(super) fn stop_queue(&mut self) -> Result<(), String> {
        if !std::mem::take(&mut self.view.processing_queue) {
            return Ok(());
        }
        if let Some(worker) = self.worker.as_ref() {
            match self.queue.pause(worker.id) {
                Ok(()) | Err(Error::Conflict) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    pub(super) fn pause(&mut self, id: JobId) -> Result<(), String> {
        self.admit(id)?;
        self.queue.pause(id).map_err(|error| error.to_string())?;
        if self.worker.as_ref().is_some_and(|worker| worker.id == id) {
            self.view.processing_queue = false;
        }
        Ok(())
    }

    pub(super) fn advance_queue(&mut self) {
        if !self.view.processing_queue || self.worker.is_some() {
            return;
        }
        match self.queue.oldest_queued(speech::KIND) {
            Ok(Some(id)) => {
                if let Err(error) = self.start(id, true) {
                    self.view.processing_queue = false;
                    self.view.notice = Some(Notice::Error(error));
                }
            }
            Ok(None) => self.view.processing_queue = false,
            Err(error) => {
                self.view.processing_queue = false;
                self.view.notice = Some(Notice::Error(error.to_string()));
            }
        }
    }
}
