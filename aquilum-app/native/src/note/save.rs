use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::sync::atomic::{AtomicBool, Ordering};

use aquilum_core::Core;
use aquilum_core::documents::conflict::{ConflictCause, ConflictReport};
use aquilum_core::documents::io::preserve_copy;
use aquilum_core::files::error::FileCommandError;
use aquilum_core::files::gate;
use aquilum_core::history::Source;

use super::read::read;

pub enum Outcome {
    Saved,
    Copied { copy: String, disk: Option<crop::Rope> },
    Unsaved,
    Failed(String),
    /// The primary file/history may have changed; never resubmit this save.
    NeedsReview(String),
}

/// Serialize all saves already queued for a single note. If any write returns
/// an uncertain result, subsequent queued jobs refuse BEFORE File Gate I/O.
/// UI reads only the atomic flag; it never waits on the disk-write mutex.
#[derive(Default)]
pub(super) struct SaveSafety {
    writer: Mutex<Option<String>>,
    needs_review: AtomicBool,
}

impl SaveSafety {
    pub(super) fn held(&self) -> bool {
        self.needs_review.load(Ordering::Acquire)
    }

    pub(super) fn run(&self, operation: impl FnOnce() -> Outcome) -> Outcome {
        let mut state = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(reason) = state.as_ref() {
            return Outcome::NeedsReview(reason.clone());
        }
        let result = operation();
        if let Outcome::NeedsReview(reason) = &result {
            *state = Some(reason.clone());
            self.needs_review.store(true, Ordering::Release);
        }
        result
    }
}

fn write(core: &Core, path: &PathBuf, text: &crop::Rope, hash: &Mutex<String>) -> Outcome {
    let _zone = aq_trace::zone("заметка: сохранение");
    let body = text.to_string();
    let expected = hash.lock().unwrap_or_else(PoisonError::into_inner).clone();
    match gate::write(core, path, &body, Some(&expected), Source::Me, None) {
        Ok(result) => {
            *hash.lock().unwrap_or_else(PoisonError::into_inner) = result.hash;
            Outcome::Saved
        }
        Err(FileCommandError::NeedsReview { message }) => Outcome::NeedsReview(message),
        Err(FileCommandError::Conflict { .. }) => {
            let report = ConflictReport { cause: ConflictCause::Diverged, disk_hash: None, synced_hash: Some(expected) };
            let offset = i64::from(chrono::Local::now().offset().local_minus_utc());
            let copy = match preserve_copy(core, path, &body, report, offset) {
                Ok(Some(copy)) => copy,
                Ok(None) => return Outcome::Unsaved,
                Err(error) => {
                    return Outcome::Failed(format!("conflict copy could not be saved: {error:?}"));
                }
            };
            let disk = read(path).ok().map(|loaded| {
                *hash.lock().unwrap_or_else(PoisonError::into_inner) = loaded.hash;
                loaded.text
            });
            Outcome::Copied { copy: copy.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), disk }
        }
        Err(error) => {
            report!("{} не сохранён: {error:?}", path.display());
            Outcome::Failed(format!("{error}"))
        }
    }
}

fn guard_save(write: impl FnOnce() -> Outcome) -> Outcome {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(write))
        // Panic location is unknown: the primary bytes might already have
        // been replaced. Retrying is unsafe even if the worker recovered.
        .unwrap_or_else(|_| Outcome::NeedsReview("background save panicked; disk state uncertain".to_owned()))
}

pub fn queue(
    core: Arc<Core>, path: PathBuf, text: crop::Rope,
    hash: Arc<Mutex<String>>, done: Sender<Outcome>,
    safety: Arc<SaveSafety>,
) {
    crate::disk::run(move || {
        // Hold one per-note serialization guard ACROSS the real File Gate and
        // HistoryService call, not just when processing the response. A second
        // already-queued save cannot race ahead of an uncertain first result.
        let outcome = safety.run(|| guard_save(|| write(&core, &path, &text, &hash)));
        let _ = done.send(outcome);
    });
}

#[cfg(test)]
mod tests {
    use super::{guard_save, Outcome, SaveSafety};
    use std::cell::Cell;

    #[test]
    fn panicking_save_is_a_non_replayable_uncertain_result() {
        let outcome = guard_save(|| panic!("intentional failing save"));
        assert!(matches!(outcome, Outcome::NeedsReview(reason) if reason.contains("disk state uncertain")));
    }

    #[test]
    fn queued_saves_are_refused_after_first_ambiguous_ack() {
        let safety = SaveSafety::default();
        let calls = Cell::new(0);
        let first = safety.run(|| {
            calls.set(calls.get() + 1);
            Outcome::NeedsReview("primary bytes may be published".into())
        });
        assert!(matches!(first, Outcome::NeedsReview(_)));
        assert!(safety.held());
        let later = safety.run(|| {
            calls.set(calls.get() + 1);
            Outcome::Saved
        });
        assert!(matches!(later, Outcome::NeedsReview(_)));
        assert_eq!(calls.get(), 1, "already queued worker must not replay File Gate");
    }
}
