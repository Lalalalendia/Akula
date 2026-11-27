use super::{Config, Manager, Notice};
use aquilum_core::background_jobs::{Spec, speech};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

#[derive(Default)]
pub(super) struct Downloads {
    waiting: VecDeque<PathBuf>,
    active: Option<Admission>,
}

struct Admission {
    path: PathBuf,
    config: Config,
    cancelled: Arc<AtomicBool>,
    handle: JoinHandle<Result<Spec, String>>,
}

impl Drop for Downloads {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Downloads {
    fn cancel(&mut self) {
        self.waiting.clear();
        if let Some(admission) = self.active.as_ref() {
            admission.cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.active.is_some() || !self.waiting.is_empty()
    }
}

impl Manager<'_> {
    pub(super) fn published_video(&mut self, path: PathBuf) {
        if self.config.auto_downloads
            && !self.downloads.waiting.contains(&path)
            && !self
                .downloads
                .active
                .as_ref()
                .is_some_and(|job| !job.cancelled.load(Ordering::Relaxed) && job.path == path)
        {
            self.downloads.waiting.push_back(path);
        }
    }

    pub(super) fn set_auto_downloads(&mut self, enabled: bool) -> Result<(), String> {
        if enabled && self.config.paths().iter().any(Option::is_none) {
            return Err("select the speech tools before enabling automatic queueing".into());
        }
        self.config.set_auto_downloads(&self.profile, enabled)?;
        if !enabled {
            self.downloads.cancel();
        }
        Ok(())
    }

    pub(super) fn poll_downloads(&mut self) {
        if self
            .downloads
            .active
            .as_ref()
            .is_some_and(|job| job.handle.is_finished())
        {
            let job = self
                .downloads
                .active
                .take()
                .expect("finished download admission");
            self.finish_admission(job);
        }
        if self.downloads.active.is_none()
            && let Some(path) = self.downloads.waiting.pop_front()
        {
            let config = self.config.clone();
            let (permitted, source) = (config.clone(), path.clone());
            let cancelled = Arc::new(AtomicBool::new(false));
            let control = cancelled.clone();
            match thread::Builder::new()
                .name("speech-download-intake".into())
                .spawn(move || prepare(&permitted, &source, &control))
            {
                Ok(handle) => {
                    self.downloads.active = Some(Admission {
                        path,
                        config,
                        cancelled,
                        handle,
                    })
                }
                Err(error) => {
                    self.view.notice = Some(Notice::DownloadError(path, error.to_string()))
                }
            }
        }
        self.view.admitting_download = self
            .downloads
            .active
            .as_ref()
            .filter(|job| !job.cancelled.load(Ordering::Relaxed))
            .map(|job| job.path.clone());
    }

    fn finish_admission(&mut self, job: Admission) {
        let result = job.handle.join();
        if job.cancelled.load(Ordering::Relaxed) {
            return;
        }
        if job.config != self.config {
            self.published_video(job.path);
            return;
        }
        let result = result
            .map_err(|_| "download admission worker terminated".to_owned())
            .and_then(|result| result)
            .and_then(|spec| self.queue.enqueue(&spec).map_err(|error| error.to_string()));
        self.view.notice = Some(match result {
            Ok(_) => {
                self.bounds.clear();
                Notice::DownloadQueued(job.path)
            }
            Err(error) => Notice::DownloadError(job.path, error),
        });
    }
}

fn prepare(config: &Config, path: &Path, cancelled: &AtomicBool) -> Result<Spec, String> {
    let tools = config.verify()?;
    let check = &mut || {
        if cancelled.load(Ordering::Relaxed) {
            Err(speech::Error::Invalid(
                "download admission was cancelled".into(),
            ))
        } else {
            Ok(())
        }
    };
    let spec = speech::prepare_enqueue(&tools, path, None, "auto", check)
        .map_err(|error| error.to_string())?;
    if Path::new(&spec.source) != path {
        return Err("downloaded file no longer matches the published library path".into());
    }
    Ok(spec)
}
