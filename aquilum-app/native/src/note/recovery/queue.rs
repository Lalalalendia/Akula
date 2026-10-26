//! Off-thread, coalesced shadow staging for a future editor integration.
//! Acknowledges durability only after stage() returned. Does not touch Vault.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use crop::Rope;
use uuid::Uuid;

use super::{checked_relative, clear_after_verified_save, inspect, invalid, stage};

struct Context {
    data: PathBuf,
    vault: PathBuf,
    workspace: Uuid,
    document: PathBuf,
    baseline: String,
}

#[derive(Default)]
struct State {
    next_revision: u64,
    pending: Option<(u64, Rope)>,
    running: bool,
    durable: Option<u64>,
    failure: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::note) struct CaptureStatus {
    pub(in crate::note) durable_revision: Option<u64>,
    pub(in crate::note) error: Option<String>,
    pub(in crate::note) pending: bool,
}

/// Intended to be armed once for a note after its File Gate source baseline
/// is verified, then driven by full Rope snapshots after each editor change.
/// The production App does not wire this in until recovery UX is available.
pub(in crate::note) struct SnapshotQueue {
    context: Arc<Context>,
    state: Arc<Mutex<State>>,
}

impl SnapshotQueue {
    pub(in crate::note) fn new(data: &Path, vault: &Path, workspace: Uuid, document: &Path, baseline: &str) -> io::Result<Self> {
        if baseline.len() != 64 || !baseline.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid File Gate baseline digest"));
        }
        checked_relative(vault, document)?;
        // Prior revisions may survive a process crash. Never reuse a revision
        // number after restart and silently overwrite a different snapshot.
        let previous = inspect(data, vault, workspace, document)?.map_or(0, |entry| entry.revision);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let stamp = u64::try_from(stamp).map_err(|_| invalid("snapshot timestamp overflow"))?;
        Ok(Self {
            context: Arc::new(Context {
                data: data.to_path_buf(),
                vault: vault.to_path_buf(),
                workspace,
                document: document.to_path_buf(),
                baseline: baseline.to_owned(),
            }),
            state: Arc::new(Mutex::new(State { next_revision: stamp.max(previous), ..State::default() })),
        })
    }

    /// Enqueue a persistent Rope copy without serializing on the GUI thread.
    /// High-frequency edits coalesce while one worker is pending or writing.
    /// The returned revision is NOT an fsync acknowledgment.
    pub(in crate::note) fn request(&self, body: Rope) -> io::Result<u64> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let revision = state.next_revision.checked_add(1)
            .ok_or_else(|| invalid("recovery revision counter exhausted"))?;
        state.next_revision = revision;
        state.pending = Some((revision, body));
        let start = !state.running;
        state.running = true;
        drop(state);
        if start {
            let context = Arc::clone(&self.context);
            let state = Arc::clone(&self.state);
            crate::disk::run(move || drain(context, state));
        }
        Ok(revision)
    }

    /// Never clear a shadow on enqueue or a guessed save: a successful File
    /// Gate response only schedules an exact disk-byte comparison in the same
    /// serial worker used for shadow creation and normal note saves.
    pub(in crate::note) fn after_save_ack(&self) {
        let context = Arc::clone(&self.context);
        let state = Arc::clone(&self.state);
        crate::disk::run(move || {
            let revision = state.lock().unwrap_or_else(PoisonError::into_inner).durable;
            if let Some(revision) = revision {
                let result = clear_after_verified_save(
                    &context.data, &context.vault, context.workspace, &context.document, revision,
                );
                if let Err(error) = result {
                    state.lock().unwrap_or_else(PoisonError::into_inner).failure =
                        Some(format!("recovery cleanup needs review: {error}"));
                }
            }
        });
    }

    pub(in crate::note) fn status(&self) -> CaptureStatus {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        CaptureStatus {
            durable_revision: state.durable,
            error: state.failure.clone(),
            pending: state.running || state.pending.is_some(),
        }
    }
}

fn drain(context: Arc<Context>, state: Arc<Mutex<State>>) {
    loop {
        let task = {
            let mut entry = state.lock().unwrap_or_else(PoisonError::into_inner);
            match entry.pending.take() {
                Some(task) => task,
                None => {
                    entry.running = false;
                    return;
                }
            }
        };
        let (revision, rope) = task;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            stage(
                &context.data,
                &context.vault,
                context.workspace,
                &context.document,
                &context.baseline,
                revision,
                &rope.to_string(),
            )
        }))
        .unwrap_or_else(|_| Err(io::Error::other("background snapshot worker panicked")));
        let mut entry = state.lock().unwrap_or_else(PoisonError::into_inner);
        match result {
            Ok(_) => {
                entry.durable = Some(revision);
                entry.failure = None;
            }
            Err(error) => {
                // Status remains observable; never mark a failed write durable.
                entry.failure = Some(format!("recovery snapshot failed: {error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc::channel;

    fn digest(path: &Path) -> String {
        blake3::hash(&fs::read(path).unwrap()).to_hex().to_string()
    }

    #[test]
    fn rapid_edit_burst_coalesces_and_only_final_bytes_become_durable() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("draft.md");
        fs::write(&doc, "before").unwrap();
        let queue = SnapshotQueue::new(data.path(), vault.path(), Uuid::new_v4(), &doc, &digest(&doc)).unwrap();
        let (tx, rx) = channel::<()>();
        // Hold the one serial disk worker to make coalescing deterministic.
        crate::disk::run(move || { rx.recv().unwrap(); });
        let first = queue.request(Rope::from("а")).unwrap();
        let last = queue.request(Rope::from("абв 😀")).unwrap();
        assert!(last > first);
        assert!(queue.status().pending);
        assert_eq!(queue.status().durable_revision, None);
        tx.send(()).unwrap();
        crate::disk::wait();
        let candidate = inspect(data.path(), vault.path(), queue.context.workspace, &doc).unwrap().unwrap();
        assert_eq!(candidate.body, "абв 😀");
        assert_eq!(candidate.revision, last);
        assert_eq!(queue.status().durable_revision, Some(last));
        assert!(!queue.status().pending);
        assert!(queue.status().error.is_none());
        assert_eq!(fs::read_to_string(doc).unwrap(), "before");
    }

    #[test]
    fn failure_never_reports_durability_or_modifies_original_note() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("fail.md");
        fs::write(&doc, "before").unwrap();
        let queue = SnapshotQueue::new(data.path(), vault.path(), Uuid::new_v4(), &doc, &digest(&doc)).unwrap();
        fs::remove_file(&doc).unwrap();
        queue.request(Rope::from("unpublished")).unwrap();
        crate::disk::wait();
        let status = queue.status();
        assert!(status.durable_revision.is_none());
        assert!(status.error.is_some());
        assert!(!status.pending);
        assert!(!doc.exists());
    }

    #[test]
    fn reopening_queue_chooses_revision_above_previous_snapshot() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("restart.md");
        fs::write(&doc, "before").unwrap();
        let workspace = Uuid::new_v4();
        let baseline = digest(&doc);
        // Simulate a prior run that left a newer-than-clock revision behind.
        let previous = u64::MAX - 2;
        stage(data.path(), vault.path(), workspace, &doc, &baseline, previous, "unsaved old").unwrap();
        let queue = SnapshotQueue::new(data.path(), vault.path(), workspace, &doc, &baseline).unwrap();
        let next = queue.request(Rope::from("unsaved new")).unwrap();
        assert_eq!(next, u64::MAX - 1);
        crate::disk::wait();
        assert_eq!(inspect(data.path(), vault.path(), workspace, &doc).unwrap().unwrap().body, "unsaved new");
        assert_eq!(fs::read_to_string(doc).unwrap(), "before");
    }
}
