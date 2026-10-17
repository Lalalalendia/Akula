use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};

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
        .unwrap_or_else(|_| Outcome::Failed("background save panicked".to_owned()))
}

pub fn queue(core: Arc<Core>, path: PathBuf, text: crop::Rope, hash: Arc<Mutex<String>>, done: Sender<Outcome>) {
    crate::disk::run(move || {
        // Always acknowledge the job, including a panic in a storage adapter.
        // Otherwise Note::queued never drains and the editor cannot retry.
        let _ = done.send(guard_save(|| write(&core, &path, &text, &hash)));
    });
}

#[cfg(test)]
mod tests {
    use super::{guard_save, Outcome};

    #[test]
    fn panicking_save_is_reported_as_failure_instead_of_losing_its_acknowledgement() {
        let outcome = guard_save(|| panic!("intentional failing save"));
        assert!(matches!(outcome, Outcome::Failed(reason) if reason == "background save panicked"));
    }
}
