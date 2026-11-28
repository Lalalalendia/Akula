use std::path::PathBuf;
mod run;
use run::serve;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::sync::mpsc::{Sender, channel};
use aquilum_core::Core;
use aquilum_core::search::models::SearchResponse;
use aquilum_core::text::cancel::{Cancel, Generation};


pub(in crate::app) struct Found {
    pub(super) generation: u64,
    pub(super) worker: u64,
    pub(super) results: Results,
}

pub(super) enum Results {
    Notes(Result<SearchResponse, String>),
    Speech(Vec<aquilum_core::background_jobs::speech::search::Hit>),
}

struct Ask {
    workspace: Option<PathBuf>,
    query: String,
    cancel: Cancel,
}

pub(super) struct Worker {
    asks: Sender<Ask>,
    generation: Generation,
    pub(super) id: u64,
}

impl Worker {
    pub(super) fn spawn(core: Arc<Core>, profile: PathBuf, found: Sender<Found>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (asks, inbox) = channel();
        let started = std::thread::Builder::new().name("поиск Ctrl+O".into()).spawn(move || serve(id, &core, &profile, &inbox, &found, &*wake));
        if let Err(error) = started {
            report!("поток поиска не запущен: {error}");
        }
        Self { asks, id, generation: Generation::default() }
    }

    pub(super) fn ask(&self, workspace: Option<PathBuf>, query: String) {
        let cancel = self.generation.next();
        let _ = self.asks.send(Ask { workspace, query, cancel });
    }

    pub(super) fn stop(&self) {
        self.generation.stop();
    }

    pub(super) fn current(&self) -> u64 {
        self.generation.current()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.generation.stop();
    }
}
