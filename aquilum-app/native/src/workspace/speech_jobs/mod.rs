//! One local speech dispatcher per open Akula profile.

mod commands;
mod config;
mod export;
mod manager;
pub(crate) mod playback;
#[cfg(test)]
mod tests;

use aquilum_core::background_jobs::Queue;
use aquilum_core::background_jobs::{JobId, State, speech::Tools};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(350);
const PAGE_SIZE: u32 = 8;
type Wake = Arc<dyn Fn() + Send + Sync>;

pub(crate) use commands::{Action, Command, Tool};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Track {
    pub(crate) index: u32,
    pub(crate) language: Option<String>,
    pub(crate) title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Selection {
    pub(crate) source: PathBuf,
    pub(crate) tracks: Vec<Track>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JobRow {
    pub(crate) id: JobId,
    pub(crate) source: PathBuf,
    pub(crate) state: State,
    pub(crate) committed: i64,
    pub(crate) total: Option<i64>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Notice {
    Error(String),
    LeaseBusy,
    Exported,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct View {
    pub(crate) tools: [Option<PathBuf>; 5],
    pub(crate) jobs: Vec<JobRow>,
    pub(crate) selection: Option<Selection>,
    pub(crate) active: Option<JobId>,
    pub(crate) caption_job: Option<JobId>,
    pub(crate) preparing: bool,
    pub(crate) loaded: bool,
    pub(crate) older: bool,
    pub(crate) newer: bool,
    pub(crate) notice: Option<Notice>,
}

impl View {
    pub(crate) fn ready(&self) -> bool {
        self.loaded
            && !self.preparing
            && self.active.is_none()
            && self.tools.iter().all(Option::is_some)
    }
}

#[derive(Default)]
struct Shared {
    view: Mutex<View>,
    updated: AtomicBool,
    playback: playback::Bridge,
}

pub(crate) struct Dispatcher {
    tx: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    finished: mpsc::Receiver<()>,
}

impl Dispatcher {
    pub(crate) fn new(profile: PathBuf, wake: Wake) -> Self {
        let (tx, rx) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let worker_view = Arc::clone(&shared);
        let worker_wake = Arc::clone(&wake);
        let page_requests = tx.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("speech-dispatch".into())
            .spawn(move || {
                manager::serve(profile, rx, page_requests, &worker_view, &worker_wake);
                let _ = done.send(());
            })
        {
            shared.publish(
                View {
                    loaded: true,
                    notice: Some(Notice::Error(error.to_string())),
                    ..View::default()
                },
                &wake,
            );
        }
        Self {
            tx,
            shared,
            finished,
        }
    }

    pub(crate) fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub(crate) fn view(&self) -> View {
        self.shared
            .view
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn take_caption_update(&self) -> Option<Option<Arc<playback::Attached>>> {
        self.shared.playback.take_update()
    }

    pub(crate) fn updated(&self) -> bool {
        self.shared.updated.swap(false, Ordering::AcqRel)
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        if self.tx.send(Command::Shutdown).is_ok() {
            let _ = self.finished.recv_timeout(Duration::from_secs(2));
        }
    }
}

impl Shared {
    fn publish(&self, view: View, wake: &Wake) {
        let mut previous = self.view.lock().unwrap_or_else(|error| error.into_inner());
        if *previous != view {
            *previous = view;
            drop(previous);
            self.updated.store(true, Ordering::Release);
            wake();
        }
    }
}
