//! One local speech dispatcher per open Akula profile.

mod commands;
mod config;
mod export;
mod manager;
mod view; pub(crate) use view::{BatchReport, JobRow, Notice, Selection, Track, View};
pub(crate) mod playback; pub(crate) mod transcript;
#[cfg(test)]
mod tests;

use aquilum_core::background_jobs::Queue;
use aquilum_core::background_jobs::{JobId, speech::Tools}; #[cfg(test)] use aquilum_core::background_jobs::State;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(350);
const PAGE_SIZE: u32 = 8;
type Wake = Arc<dyn Fn() + Send + Sync>;

pub(crate) use commands::{Action, Command, Tool};

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
