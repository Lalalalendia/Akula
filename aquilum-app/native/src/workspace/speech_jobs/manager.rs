mod intake; mod transcript;
use super::config::Config;
use super::playback::Attached;
use super::{
    Action, Command, JobId, JobRow, Notice, PAGE_SIZE, POLL, Selection, Shared, State, Track, View,
    Wake,
};
use aquilum_core::background_jobs::{
    Queue,
    speech::{self, Outcome},
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};

struct Worker {
    id: JobId,
    handle: JoinHandle<Result<Outcome, String>>,
}

struct Manager<'a> {
    queue: Queue,
    profile: PathBuf,
    database: PathBuf,
    config: Config,
    worker: Option<Worker>,
    bounds: Vec<JobId>,
    view: View,
    shared: &'a Shared,
    page_requests: Sender<Command>,
    wake: &'a Wake,
}

pub(super) fn serve(
    profile: PathBuf,
    commands: Receiver<Command>,
    page_requests: Sender<Command>,
    shared: &Shared,
    wake: &Wake,
) {
    let database = profile.join("media-jobs.sqlite");
    let queue = match Queue::open(&database) {
        Ok(queue) => queue,
        Err(error) => {
            shared.publish(
                View {
                    loaded: true,
                    notice: Some(Notice::Error(error.to_string())),
                    ..View::default()
                },
                wake,
            );
            return;
        }
    };
    let mut manager = Manager {
        queue,
        profile,
        database,
        config: Config::default(),
        worker: None,
        bounds: Vec::new(),
        view: View {
            loaded: true,
            ..View::default()
        },
        shared,
        page_requests,
        wake,
    };
    match Config::load(&manager.profile) {
        Ok(config) => manager.config = config,
        Err(error) => manager.view.notice = Some(Notice::Error(error)),
    }
    manager.publish();
    loop {
        let command = if manager.worker.is_some() {
            match commands.recv_timeout(POLL) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };
        if let Some(command) = command {
            if matches!(command, Command::Shutdown) {
                break;
            }
            if !matches!(command, Command::CaptionPage(..) | Command::Transcript(_)) {
                manager.view.notice = None;
            }
            if let Err(error) = manager.handle(command) {
                manager.view.notice = Some(Notice::Error(error));
            }
        }
        manager.reap();
        manager.refresh_captions();
        manager.publish();
    }
    if let Some(worker) = manager.worker.take() {
        let _ = manager.queue.pause(worker.id);
        let _ = worker.handle.join();
    }
}

impl Manager<'_> {
    fn publish(&mut self) {
        self.view.tools = self.config.paths();
        self.view.newer = !self.bounds.is_empty();
        self.view.active = self.worker.as_ref().map(|worker| worker.id);
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

    fn handle(&mut self, command: Command) -> Result<(), String> {
        match command {
            Command::SetTool(tool, path) => self.prepare(|this| {
                this.config.choose(&this.profile, tool, path)?;
                this.view.selection = None;
                Ok(())
            }),
            Command::Inspect(source) => self.prepare(|this| this.inspect(source)),
            Command::Export(id, path) => self.prepare(|this| {
                this.admit(id)?;
                super::export::save(&this.queue, id, &path)?;
                this.view.notice = Some(Notice::Exported);
                Ok(())
            }),
            Command::CaptionPage(track, page) => {
                if self
                    .shared
                    .playback
                    .selected()
                    .is_some_and(|active| Arc::ptr_eq(&active, &track))
                {
                    self.caption_update(track.load(&self.queue, page));
                }
                Ok(())
            }
            Command::Transcript(request) => { self.transcript(request); Ok(()) }
            Command::Action(Action::ShowCaptions(id)) => self.prepare(|this| {
                let track = Attached::new(&this.queue, id, this.page_requests.clone())?;
                this.shared
                    .playback
                    .select(Some(Arc::new(track)), this.wake);
                Ok(())
            }),
            Command::Action(Action::HideCaptions) => {
                self.shared.playback.select(None, self.wake);
                Ok(())
            }
            Command::Action(Action::UseTrack(track)) => self.prepare(|this| this.add(track)),
            Command::Action(Action::Run(id)) => self.start(id),
            Command::Action(Action::Pause(id)) => {
                self.admit(id)?;
                self.queue.pause(id).map_err(|error| error.to_string())
            }
            Command::Action(Action::Cancel(id)) => {
                self.admit(id)?;
                self.queue.cancel(id).map_err(|error| error.to_string())
            }
            Command::Action(Action::DismissMedia) => {
                self.view.selection = None;
                Ok(())
            }
            Command::Action(Action::Older) => {
                if self.view.older
                    && let Some(last) = self.view.jobs.last()
                {
                    self.bounds.push(last.id);
                }
                Ok(())
            }
            Command::Action(Action::Newer) => {
                self.bounds.pop();
                Ok(())
            }
            Command::Action(Action::Refresh | Action::OpenTranscript(_) | Action::PickTool(_) | Action::PickMedia | Action::Export(_))
            | Command::Shutdown => Ok(()),
        }
    }

    fn refresh_captions(&mut self) {
        if let Some(track) = self.shared.playback.selected() {
            self.caption_update(track.update_if_advanced(&self.queue));
        }
    }

    fn caption_update(&mut self, result: Result<bool, String>) {
        match result {
            Ok(true) => self.shared.playback.notify(self.wake),
            Ok(false) => (),
            Err(error) => {
                self.shared.playback.select(None, self.wake);
                self.view.notice = Some(Notice::Error(error));
            }
        }
    }

    fn admit(&self, id: JobId) -> Result<State, String> {
        self.queue
            .get(id)
            .map_err(|error| error.to_string())?
            .filter(|job| job.spec.kind == speech::KIND)
            .map(|job| job.state)
            .ok_or_else(|| "speech job is unavailable".into())
    }

    fn start(&mut self, id: JobId) -> Result<(), String> {
        if self.worker.is_some() {
            return Err("a speech job is already active".into());
        }
        match self.admit(id)? {
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
            .spawn(move || run(&database, &config, id))
            .map_err(|error| error.to_string())?;
        self.worker = Some(Worker { id, handle });
        Ok(())
    }

    fn reap(&mut self) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.handle.is_finished())
        {
            let worker = self.worker.take().expect("finished speech worker");
            match worker.handle.join() {
                Ok(Ok(Outcome::Idle)) if self.admit(worker.id) == Ok(State::Running) => {
                    self.view.notice = Some(Notice::LeaseBusy);
                }
                Ok(Ok(Outcome::Idle)) => (),
                Ok(Ok(Outcome::Completed | Outcome::Interrupted)) => (),
                Ok(Err(error)) => self.view.notice = Some(Notice::Error(error)),
                Err(_) => self.view.notice = Some(Notice::Error("speech worker terminated".into())),
            }
        }
    }
}

fn run(database: &Path, config: &Config, id: JobId) -> Result<Outcome, String> {
    let mut queue = Queue::open(database).map_err(|error| error.to_string())?;
    let tools = config.verify()?;
    let threads = thread::available_parallelism().map_or(1, |cores| cores.get().min(2) as u8);
    speech::work(&mut queue, &tools, id, threads).map_err(|error| error.to_string())
}
