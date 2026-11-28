use super::{Manager, Notice};
use aquilum_core::background_jobs::{
    Job, JobId, Queue, State,
    speech::{self, Outcome},
};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

pub(super) struct Worker {
    pub(super) id: JobId,
    pub(super) source: PathBuf,
    pub(super) handle: JoinHandle<Result<Outcome, String>>,
}

impl Manager<'_> {
    pub(super) fn job(&self, id: JobId) -> Result<Job, String> {
        self.queue
            .get(id)
            .map_err(|error| error.to_string())?
            .filter(|job| job.spec.kind == speech::KIND)
            .ok_or_else(|| "speech job is unavailable".into())
    }

    pub(super) fn start(&mut self, id: JobId, queued_only: bool) -> Result<(), String> {
        if self.installation.is_some() {
            return Err("speech toolkit installation is in progress".into());
        }
        if self.worker.is_some() {
            return Err("a speech job is already active".into());
        }
        let job = self.job(id)?;
        if queued_only && job.state != State::Queued {
            return Ok(());
        }
        match job.state {
            State::Completed | State::Cancelled => return Ok(()),
            State::Paused | State::Failed => {
                self.queue.resume(id).map_err(|error| error.to_string())?;
            }
            _ => {}
        }
        let database = self.database.clone();
        let config = self.config.clone();
        let handle = thread::Builder::new()
            .name("speech-recognition".into())
            .spawn(move || run(&database, &config, id, queued_only))
            .map_err(|error| error.to_string())?;
        self.worker = Some(Worker {
            id,
            source: PathBuf::from(job.spec.source),
            handle,
        });
        Ok(())
    }

    pub(super) fn reap(&mut self) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.handle.is_finished())
        {
            let worker = self.worker.take().expect("finished speech worker");
            match worker.handle.join() {
                Ok(Ok(Outcome::Idle))
                    if matches!(self.admit(worker.id), Ok(State::Running | State::Queued)) =>
                {
                    self.view.processing_queue = false;
                    self.view.notice = Some(Notice::LeaseBusy);
                }
                Ok(Ok(Outcome::Idle)) => (),
                Ok(Ok(Outcome::Completed | Outcome::Interrupted)) => (),
                Ok(Err(error)) => {
                    // A media failure is recorded on its own row. A startup
                    // failure can leave it queued and would cause a retry loop.
                    if self.admit(worker.id) != Ok(State::Failed) {
                        self.view.processing_queue = false;
                    }
                    self.view.notice = Some(Notice::Error(error));
                }
                Err(_) => {
                    self.view.processing_queue = false;
                    self.view.notice = Some(Notice::Error("speech worker terminated".into()));
                }
            }
        }
    }
}

fn run(
    database: &Path,
    config: &super::Config,
    id: JobId,
    queued_only: bool,
) -> Result<Outcome, String> {
    let mut queue = Queue::open(database).map_err(|error| error.to_string())?;
    let tools = config.verify()?;
    let threads = thread::available_parallelism().map_or(1, |cores| cores.get().min(2) as u8);
    let work = if queued_only {
        speech::work_queued
    } else {
        speech::work
    };
    work(&mut queue, &tools, id, threads).map_err(|error| error.to_string())
}
