//! Shared download lifecycle; each tool owns an independent state instance.
use super::Status;
use std::{collections::HashMap, path::{Path, PathBuf}, sync::{Arc, Mutex, OnceLock, atomic::{AtomicU64, Ordering}}};

pub(super) struct ToolState {
    states: OnceLock<Mutex<HashMap<PathBuf, Status>>>,
    pub(super) revision: AtomicU64,
}

impl ToolState {
    pub(super) const fn new() -> Self {
        Self { states: OnceLock::new(), revision: AtomicU64::new(0) }
    }

    pub(super) fn states(&self) -> &Mutex<HashMap<PathBuf, Status>> {
        self.states.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(super) fn revision(&self) -> u64 { self.revision.load(Ordering::Relaxed) }

    pub(super) fn set_state(&self, key: &Path, state: Status, wake: &Arc<dyn Fn() + Send + Sync>) {
        self.states().lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_path_buf(), state);
        self.revision.fetch_add(1, Ordering::Relaxed);
        wake();
    }

    pub(super) fn start(&'static self, profile: PathBuf, wake: Arc<dyn Fn() + Send + Sync>,
        status_disk: fn(&Path) -> Status,
        fetch: impl FnOnce(&Path, &Arc<dyn Fn() + Send + Sync>) -> Result<(), String> + Send + 'static,
        [busy, installed]: [&str; 2],
    ) -> Result<(), String> {
        {
            let mut lock = self.states().lock().unwrap_or_else(|e| e.into_inner());
            if matches!(lock.get(&profile), Some(Status::Downloading(_))) {
                return Err(busy.into());
            }
            if matches!(status_disk(&profile), Status::Installed) {
                return Err(installed.into());
            }
            lock.insert(profile.clone(), Status::Downloading(0));
        }
        self.revision.fetch_add(1, Ordering::Relaxed);
        wake();
        std::thread::spawn(move || {
            let next = match fetch(&profile, &wake) {
                Ok(()) => Status::Installed,
                Err(reason) => Status::Failed(reason),
            };
            self.set_state(&profile, next, &wake);
        });
        Ok(())
    }
}
