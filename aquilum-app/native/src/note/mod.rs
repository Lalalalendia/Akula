mod read;
mod save;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use aquilum_core::Core;
use aquilum_core::files::error::FileCommandError;

use crate::i18n::{t, t_with};
pub use read::read;
use save::Outcome;

const POLL: Duration = Duration::from_millis(20);
const RETRY: Duration = Duration::from_secs(5);

// Exactly one probe per open note. Watcher bursts coalesce into a fresh probe.
struct ReloadJob {
    done: Receiver<Option<read::Loaded>>,
    revision: u64,
    path: PathBuf,
    again: bool,
    cancelled: Arc<AtomicBool>,
}

impl Drop for ReloadJob {
    fn drop(&mut self) {
        // Dropping a tab prevents its worker from decoding a stale huge file.
        self.cancelled.store(true, Ordering::Release);
    }
}

pub struct Note {
    pub path: PathBuf,
    hash: Arc<Mutex<String>>,
    pending: Option<Instant>,
    pub save_error: Option<String>,
    replaced: Option<crop::Rope>,
    queued: usize,
    done: (Sender<Outcome>, Receiver<Outcome>),
    revision: u64,
    reload_job: Option<ReloadJob>,
}

impl Note {
    pub fn open(path: PathBuf) -> Result<(Note, crop::Rope), FileCommandError> {
        let loaded = read(&path)?;
        let note = Note { path, hash: Arc::new(Mutex::new(loaded.hash)), pending: None, save_error: None, replaced: None, queued: 0, done: channel(), revision: 0, reload_job: None };
        Ok((note, loaded.text))
    }

    pub fn edit(&mut self, debounce: Duration) {
        self.revision = self.revision.wrapping_add(1);
        self.pending = Some(Instant::now() + debounce);
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.queued > 0 { Some(Instant::now() + POLL) } else { self.pending }
    }

    fn queue(&mut self, core: &Arc<Core>, text: &crop::Rope) {
        self.revision = self.revision.wrapping_add(1);
        self.pending = None;
        self.queued += 1;
        save::queue(Arc::clone(core), self.path.clone(), text.clone(), Arc::clone(&self.hash), self.done.0.clone());
    }

    pub fn tick(&mut self, core: &Arc<Core>, text: &crop::Rope, now: Instant) -> bool {
        let before = self.save_error.clone();
        self.collect();
        if self.pending.is_some_and(|due| due <= now) {
            self.queue(core, text);
        }
        self.save_error != before
    }

    fn collect(&mut self) {
        while let Ok(outcome) = self.done.1.try_recv() {
            self.queued -= 1;
            match outcome {
                Outcome::Saved => self.save_error = None,
                Outcome::Copied { copy, disk } => {
                    self.replaced = disk;
                    self.save_error = Some(t_with("editor.conflictCopy", &[("name", &copy)]));
                }
                Outcome::Unsaved => self.fail(t("editor.conflictUnsaved")),
                Outcome::Failed(reason) => self.fail(t_with("editor.saveFailed", &[("reason", &reason)])),
            }
        }
    }

    fn fail(&mut self, message: String) {
        self.pending = Some(Instant::now() + RETRY);
        self.save_error = Some(message);
    }

    pub fn flush(&mut self, core: &Arc<Core>, text: &crop::Rope) {
        if self.pending.is_some() {
            self.queue(core, text);
        }
        crate::disk::wait();
        self.collect();
    }

    pub fn leave(mut self, core: &Arc<Core>, text: &crop::Rope) {
        if self.pending.is_some() {
            self.queue(core, text);
        }
    }

    pub fn take_replaced(&mut self) -> Option<crop::Rope> {
        self.replaced.take()
    }

    pub fn retry(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.pending = Some(Instant::now());
    }

    /// Hash and read changed files away from the window event loop.
    pub fn request_reload(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        if self.pending.is_some() || self.queued > 0 {
            return;
        }
        if let Some(job) = self.reload_job.as_mut() {
            job.again = true;
            return;
        }
        let expected = self.hash.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let path = self.path.clone();
        let revision = self.revision;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        let (tx, done) = channel();
        let worker_path = path.clone();
        let spawned = std::thread::Builder::new().name("akula-note-reload".into()).spawn(move || {
            // A panic must still settle the receiver instead of wedging reload.
            let result = std::panic::catch_unwind(|| {
                let probe = aquilum_core::files::document::read_file_hash_impl(&worker_path).ok()?;
                if stop.load(Ordering::Acquire) || probe == expected {
                    return None;
                }
                let loaded = read(&worker_path).ok()?;
                if stop.load(Ordering::Acquire) || loaded.hash == expected {
                    return None;
                }
                Some(loaded)
            }).ok().flatten();
            if tx.send(result).is_ok() {
                wake();
            }
        });
        if let Err(error) = spawned {
            report!("фоновое чтение {} не запущено: {error}", path.display());
            return;
        }
        self.reload_job = Some(ReloadJob { done, revision, path, again: false, cancelled });
    }

    /// Called on a wake: only accept results for the same note revision/path.
    pub fn take_reload(&mut self, wake: &Arc<dyn Fn() + Send + Sync>) -> Option<crop::Rope> {
        let job = self.reload_job.take()?;
        let loaded = match job.done.try_recv() {
            Ok(loaded) => loaded,
            Err(TryRecvError::Empty) => {
                self.reload_job = Some(job);
                return None;
            }
            Err(TryRecvError::Disconnected) => return None,
        };
        if job.again {
            self.request_reload(Arc::clone(wake));
            return None;
        }
        if job.path != self.path || job.revision != self.revision || self.pending.is_some() || self.queued > 0 {
            return None;
        }
        let loaded = loaded?;
        let mut hash = self.hash.lock().unwrap_or_else(PoisonError::into_inner);
        if loaded.hash == *hash {
            return None;
        }
        *hash = loaded.hash;
        Some(loaded.text)
    }

    pub fn is(&self, path: &Path) -> bool {
        aquilum_core::search::paths::same_path(&self.path, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_ignores_own_write_and_sees_external_one() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("a.md");
        std::fs::write(&path, "раз").unwrap();
        let (mut note, text) = Note::open(path.clone()).unwrap();
        assert_eq!(text, "раз");
        assert_eq!(note.reload(), None);
        std::fs::write(&path, "два").unwrap();
        assert_eq!(note.reload().map(|text| text.to_string()).as_deref(), Some("два"));
        note.edit(Duration::ZERO);
        assert!(note.deadline().is_some());
        assert_eq!(note.reload(), None, "несохранённая правка не затирается");
    }

    #[test]
    fn reload_detects_same_size_external_edit_with_restored_mtime() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("same-stat.md");
        std::fs::write(&path, "раз").unwrap();
        let (mut note, original) = Note::open(path.clone()).unwrap();
        let previous = std::fs::metadata(&path).unwrap();
        let modified = previous.modified().unwrap();
        std::fs::write(&path, "два").unwrap();
        std::fs::File::open(&path).unwrap().set_modified(modified).unwrap();

        let restored = std::fs::metadata(&path).unwrap();
        assert_eq!(restored.len(), previous.len(), "same byte length is essential");
        assert_eq!(restored.modified().unwrap(), modified, "timestamp must not signal a change");
        assert_eq!(original.to_string(), "раз");
        assert_eq!(note.reload().map(|rope| rope.to_string()).as_deref(), Some("два"));
        assert!(note.reload().is_none(), "unchanged bytes must not replace the Rope again");
    }

    #[test]
    fn identical_external_rewrite_does_not_reload_or_override_pending_changes() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("unchanged.md");
        std::fs::write(&path, "текст").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        std::fs::write(&path, "текст").unwrap();
        assert!(note.reload().is_none(), "byte-identical external rewrite is a no-op");
        note.edit(Duration::ZERO);
        std::fs::write(&path, "иначе").unwrap();
        assert!(note.reload().is_none(), "pending edits must never be replaced");
    }
}
