use super::{Manager, Notice};
use crate::workspace::speech_jobs::toolkit;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::thread::{self, JoinHandle};

pub(super) struct Installation {
    progress: Arc<AtomicU8>,
    worker: JoinHandle<Result<tempfile::TempDir, String>>,
}

impl Manager<'_> {
    pub(super) fn install_tools(&mut self) -> Result<(), String> {
        if self.installation.is_some() {
            return Ok(());
        }
        if self.worker.is_some() || self.view.processing_queue {
            return Err("stop speech recognition before installing the toolkit".into());
        }
        if !toolkit::supported() {
            return Err("automatic speech setup requires Windows 10 or later (x64)".into());
        }
        let profile = self.profile.clone();
        let progress = Arc::new(AtomicU8::new(0));
        let worker_progress = Arc::clone(&progress);
        let worker = thread::Builder::new()
            .name("speech-toolkit-install".into())
            .spawn(move || {
                toolkit::download(&profile, |percent| {
                    worker_progress.store(percent, Ordering::Relaxed);
                })
            })
            .map_err(|error| error.to_string())?;
        self.installation = Some(Installation { progress, worker });
        Ok(())
    }

    pub(super) fn poll_installation(&mut self) {
        let Some(install) = &self.installation else {
            return;
        };
        if !install.worker.is_finished() {
            self.view.installing_tools = Some(install.progress.load(Ordering::Relaxed));
            return;
        }
        let install = self
            .installation
            .take()
            .expect("finished toolkit installation");
        self.view.installing_tools = None;
        let result = install
            .worker
            .join()
            .map_err(|_| "speech toolkit installation worker terminated".to_owned())
            .and_then(|install| install)
            .and_then(|install| self.config.use_installed(&self.profile, install));
        self.view.notice = Some(match result {
            Ok(()) => {
                self.view.selection = None;
                Notice::ToolkitInstalled
            }
            Err(error) => Notice::Error(error),
        });
    }
}
