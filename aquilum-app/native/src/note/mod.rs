mod bridge;
mod read;
mod recovery;
mod save;
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicBool, Ordering}, mpsc::{Receiver, Sender, TryRecvError, channel}, Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use aquilum_core::Core;
use aquilum_core::files::error::FileCommandError;

use crate::i18n::{t, t_with};
pub use read::read;
use save::{Outcome, SaveSafety};

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
    save_safety: Arc<SaveSafety>,
    revision: u64,
    reload_job: Option<ReloadJob>,
    shadow: Option<recovery::SnapshotQueue>,
}

impl Note {
    pub fn open(path: PathBuf) -> Result<(Note, crop::Rope), FileCommandError> {
        let loaded = read(&path)?;
        Ok(Self::from_verified_cache(path, loaded.text, loaded.hash))
    }

    /// Construct an ordinary clean note only after a raw on-disk BLAKE3 match.
    /// The caller must compare the cached digest to the *current* file bytes.
    pub(crate) fn from_verified_cache(path: PathBuf, text: crop::Rope, hash: String) -> (Note, crop::Rope) {
        let note = Note { path, hash: Arc::new(Mutex::new(hash)), pending: None, save_error: None, replaced: None, queued: 0, done: channel(), save_safety: Arc::new(SaveSafety::default()), revision: 0, reload_job: None, shadow: None };
        (note, text)
    }

    /// Do not retain a snapshot that could still be saved, conflicted or
    /// replaced. File freshness is checked again on every cache lookup.
    pub(crate) fn clean_cache_hash(&self) -> Option<String> {
        if self.pending.is_some() || self.queued > 0 || self.save_error.is_some() || self.replaced.is_some() {
            return None;
        }
        Some(self.hash.lock().unwrap_or_else(PoisonError::into_inner).clone())
    }

    pub fn edit(&mut self, debounce: Duration) {
        self.revision = self.revision.wrapping_add(1);
        // Keep new edits in memory after an ambiguous result; never schedule
        // another native write until explicit, separately reviewed recovery.
        self.pending = (!self.save_safety.held()).then(|| Instant::now() + debounce);
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.queued > 0 { Some(Instant::now() + POLL) }
        else if self.save_safety.held() { None }
        else { self.pending }
    }

    fn queue(&mut self, core: &Arc<Core>, text: &crop::Rope) {
        self.revision = self.revision.wrapping_add(1);
        self.pending = None;
        self.queued += 1;
        save::queue(
            Arc::clone(core), self.path.clone(), text.clone(),
            Arc::clone(&self.hash), self.done.0.clone(), Arc::clone(&self.save_safety),
        );
    }

    pub fn tick(&mut self, core: &Arc<Core>, text: &crop::Rope, now: Instant) -> bool {
        let before = self.save_error.clone();
        self.collect();
        if !self.save_safety.held() && self.pending.is_some_and(|due| due <= now) {
            self.queue(core, text);
        }
        self.save_error != before
    }

    fn collect(&mut self) {
        while let Ok(outcome) = self.done.1.try_recv() {
            self.queued -= 1;
            // Worker-side hold is authoritative and can already be set
            // before a previously queued success ACK is delivered. A stale
            // success must not clear the manual-review status.
            if self.save_safety.held() {
                self.pending = None;
                let reason = match outcome {
                    Outcome::NeedsReview(reason) => reason,
                    _ => "native save needs review before any retry".to_owned(),
                };
                self.save_error = Some(t_with("editor.saveFailed", &[("reason", &reason)]));
                continue;
            }
            match outcome {
                Outcome::Saved => { self.save_error = None; self.saved_ack(); },
                Outcome::Copied { copy, disk } => {
                    self.replaced = disk;
                    self.save_error = Some(t_with("editor.conflictCopy", &[("name", &copy)]));
                }
                Outcome::Unsaved => self.fail(t("editor.conflictUnsaved")),
                Outcome::Failed(reason) => self.fail(t_with("editor.saveFailed", &[("reason", &reason)])),
                Outcome::NeedsReview(reason) => {
                    self.pending = None;
                    self.save_error = Some(t_with("editor.saveFailed", &[("reason", &reason)]));
                },
            }
        }
    }

    fn fail(&mut self, message: String) {
        if !self.save_safety.held() {
            self.pending = Some(Instant::now() + RETRY);
        }
        self.save_error = Some(message);
    }

    pub fn flush(&mut self, core: &Arc<Core>, text: &crop::Rope) {
        if self.pending.is_some() && !self.save_safety.held() {
            self.queue(core, text);
        }
        crate::disk::wait();
        self.collect();
    }

    pub fn leave(mut self, core: &Arc<Core>, text: &crop::Rope) {
        if self.pending.is_some() && !self.save_safety.held() {
            self.queue(core, text);
        }
    }

    pub fn take_replaced(&mut self) -> Option<crop::Rope> {
        self.replaced.take()
    }

    pub fn retry(&mut self) {
        // Manual retry is NOT review/clearance for a possibly published write.
        if self.save_safety.held() { return; }
        self.revision = self.revision.wrapping_add(1);
        self.pending = Some(Instant::now());
    }

    /// Hash and read changed files away from the window event loop.
    pub fn request_reload(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        if self.save_safety.held() || self.pending.is_some() || self.queued > 0 {
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
        if job.path != self.path || job.revision != self.revision || self.save_safety.held() || self.pending.is_some() || self.queued > 0 {
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

    /// Explicit history restore/revert is a user-initiated operation that
    /// must update the visible text immediately after a successful disk write.
    /// It is not a watcher notification; bypass the async hash-only probe.
    pub fn reload_after_restore(&mut self) -> Option<crop::Rope> {
        if self.save_safety.held() || self.pending.is_some() || self.queued > 0 {
            return None;
        }
        self.revision = self.revision.wrapping_add(1);
        self.reload_job = None; // discard and cancel any older watcher result
        let loaded = read(&self.path).ok()?;
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

    fn waker() -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }

    fn wait_reload(note: &mut Note, wake: &Arc<dyn Fn() + Send + Sync>) -> Option<crop::Rope> {
        for _ in 0..400 {
            let text = note.take_reload(wake);
            if note.reload_job.is_none() {
                return text;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("background note reload did not finish");
    }


    #[test]
    fn ambiguous_native_save_never_schedules_retry_or_reload() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("hold.md");
        std::fs::write(&path, "original").unwrap();
        let (mut note, _body) = Note::open(path.clone()).unwrap();
        note.edit(Duration::ZERO);
        assert!(note.pending.is_some());

        // Simulate worker publication with an uncertain postflight. The
        // worker records HOLD before delivering its acknowledgement.
        let first = note.save_safety.run(|| Outcome::NeedsReview("lost ACK".into()));
        note.done.0.send(first).unwrap();
        note.queued = 1;
        note.collect();
        assert!(note.save_safety.held());
        assert!(note.pending.is_none());
        assert!(note.save_error.is_some());

        note.edit(Duration::ZERO);
        note.retry();
        assert!(note.pending.is_none());
        assert!(note.deadline().is_none());

        note.request_reload(waker());
        assert!(note.reload_job.is_none());
        assert!(note.reload_after_restore().is_none());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    }

    #[test]
    fn reload_ignores_own_write_and_sees_external_one() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("a.md");
        std::fs::write(&path, "раз").unwrap();
        let (mut note, text) = Note::open(path.clone()).unwrap();
        let wake = waker();
        assert_eq!(text, "раз");
        note.request_reload(Arc::clone(&wake));
        assert!(wait_reload(&mut note, &wake).is_none());
        std::fs::write(&path, "два").unwrap();
        note.request_reload(Arc::clone(&wake));
        assert_eq!(wait_reload(&mut note, &wake).map(|text| text.to_string()).as_deref(), Some("два"));
        note.edit(Duration::ZERO);
        assert!(note.deadline().is_some());
        note.request_reload(Arc::clone(&wake));
        assert!(note.reload_job.is_none(), "unsaved text must not be overwritten");
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
        assert_eq!(restored.len(), previous.len());
        assert_eq!(restored.modified().unwrap(), modified);
        assert_eq!(original.to_string(), "раз");
        let wake = waker();
        note.request_reload(Arc::clone(&wake));
        assert_eq!(wait_reload(&mut note, &wake).map(|rope| rope.to_string()).as_deref(), Some("два"));
        note.request_reload(Arc::clone(&wake));
        assert!(wait_reload(&mut note, &wake).is_none(), "unchanged bytes are a no-op");
    }

    #[test]
    fn identical_external_rewrite_does_not_reload_or_override_pending_changes() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("unchanged.md");
        std::fs::write(&path, "текст").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        std::fs::write(&path, "текст").unwrap();
        let wake = waker();
        note.request_reload(Arc::clone(&wake));
        assert!(wait_reload(&mut note, &wake).is_none());
        note.edit(Duration::ZERO);
        std::fs::write(&path, "иначе").unwrap();
        note.request_reload(Arc::clone(&wake));
        assert!(note.reload_job.is_none(), "pending edits block external reload");
    }

    #[test]
    fn late_result_cannot_overwrite_an_edit_even_after_debounce_is_cleared() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("late.md");
        std::fs::write(&path, "исходно").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        let original = note.hash.lock().unwrap().clone();
        let (tx, done) = channel();
        note.reload_job = Some(ReloadJob {
            done, revision: note.revision, path, again: false, cancelled: Arc::new(AtomicBool::new(false)),
        });
        note.edit(Duration::ZERO);
        // Even if saving completed and the pending flag cleared, revision is stale.
        note.pending = None;
        tx.send(Some(read::Loaded { text: crop::Rope::from("не принимать"), hash: "старый результат".into() })).unwrap();
        assert!(note.take_reload(&waker()).is_none());
        assert_eq!(*note.hash.lock().unwrap(), original);
    }

    #[test]
    fn repeated_events_coalesce_and_reprobe_before_publishing() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("burst.md");
        std::fs::write(&path, "первый").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        let original = note.hash.lock().unwrap().clone();
        let wake = waker();
        let (tx, done) = channel();
        note.reload_job = Some(ReloadJob {
            done, revision: note.revision, path: path.clone(), again: false, cancelled: Arc::new(AtomicBool::new(false)),
        });
        for _ in 0..30 {
            note.request_reload(Arc::clone(&wake));
        }
        assert!(note.reload_job.as_ref().unwrap().again, "burst must coalesce");
        std::fs::write(&path, "второй").unwrap();
        tx.send(Some(read::Loaded { text: crop::Rope::from("устарело"), hash: "вчера".into() })).unwrap();
        assert!(note.take_reload(&wake).is_none(), "stale intermediate result must be ignored");
        assert_eq!(*note.hash.lock().unwrap(), original);
        assert!(note.reload_job.is_some(), "one new background probe should be started");
        assert_eq!(wait_reload(&mut note, &wake).map(|r| r.to_string()).as_deref(), Some("второй"));
    }

    #[test]
    fn reload_result_for_renamed_path_is_discarded() {
        let dir = crate::test_dir::tempdir();
        let path = dir.path().join("before.md");
        std::fs::write(&path, "before").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        let (tx, done) = channel();
        note.reload_job = Some(ReloadJob {
            done, revision: note.revision, path: path.clone(), again: false, cancelled: Arc::new(AtomicBool::new(false)),
        });
        note.path = dir.path().join("after.md");
        tx.send(Some(read::Loaded { text: crop::Rope::from("stale"), hash: "stale".into() })).unwrap();
        assert!(note.take_reload(&waker()).is_none(), "renamed note must not receive old-path bytes");
    }
}
