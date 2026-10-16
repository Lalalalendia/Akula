use super::error::SearchError;
use super::index::SearchIndex;
use super::progress::IndexProgress;
use super::sync::IndexSynchronizer;
use super::models::IndexRevision;
use super::IndexNotifier;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const EVENT_QUEUE_CAPACITY: usize = 2_048;
const EVENT_BATCH_WINDOW: Duration = Duration::from_millis(80);
const EVENT_QUIET_WINDOW: Duration = Duration::from_millis(20);
const WRITER_IDLE: Duration = Duration::from_secs(5);

pub struct WorkerHandle {
    sender: mpsc::SyncSender<WorkerMessage>,
    dirty: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

enum WorkerMessage {
    Paths(Vec<PathBuf>),
    Rescan(Vec<PathBuf>),
}

struct WorkerContext {
    root: PathBuf,
    index: Arc<SearchIndex>,
    metadata_path: PathBuf,
    dirty: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    progress: Arc<IndexProgress>,
    generation: u64,
    index_notifier: IndexNotifier,
}

impl WorkerHandle {
    pub fn spawn(
        root: PathBuf,
        index: Arc<SearchIndex>,
        metadata_path: PathBuf,
        progress: Arc<IndexProgress>,
        generation: u64,
        index_notifier: IndexNotifier,
    ) -> Result<Self, SearchError> {
        let (sender, receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let dirty = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let context = WorkerContext {
            root,
            index,
            metadata_path,
            dirty: Arc::clone(&dirty),
            cancelled: Arc::clone(&cancelled),
            progress: Arc::clone(&progress),
            generation,
            index_notifier,
        };
        let thread = std::thread::Builder::new()
            .name("aquilum-search-index".to_owned())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&context, receiver)));
                let failure = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error),
                    Err(panic) => Some(SearchError::Task {
                        message: format!("поток индексации упал: {}", panic_message(panic.as_ref())),
                    }),
                };
                if let Some(error) = failure {
                    progress.failed(&error);
                }
            })
            .map_err(|error| SearchError::Task {
                message: error.to_string(),
            })?;
        Ok(Self {
            sender,
            dirty,
            cancelled,
            thread: Some(thread),
        })
    }

    pub fn queue_paths(&self, paths: Vec<PathBuf>) {
        enqueue(&self.sender, &self.dirty, WorkerMessage::Paths(paths));
    }

    pub fn queue_rescan(&self, paths: Vec<PathBuf>) {
        enqueue(&self.sender, &self.dirty, WorkerMessage::Rescan(paths));
    }

    pub fn stop(mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    context: &WorkerContext,
    receiver: mpsc::Receiver<WorkerMessage>,
) -> Result<(), SearchError> {
    let mut synchronizer = IndexSynchronizer::new(
        context.root.clone(),
        Arc::clone(&context.index),
        &context.metadata_path,
        Arc::clone(&context.progress),
    )?;
    synchronizer.full_scan(&context.cancelled)?;
    notify_index(context);
    let mut last_write = Instant::now();

    while !context.cancelled.load(Ordering::Relaxed) {
        if context.dirty.swap(false, Ordering::Relaxed) {
            synchronizer.full_scan(&context.cancelled)?;
            notify_index(context);
            last_write = Instant::now();
            continue;
        }
        match receiver.recv_timeout(Duration::from_millis(300)) {
            Ok(message) => {
                let (paths, rescan) = collect_batch(message, &receiver);
                if rescan {
                    synchronizer.full_scan(&context.cancelled)?;
                    notify_index(context);
                } else if synchronizer.apply_paths(paths)? {
                    notify_index(context);
                }
                last_write = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if synchronizer.holds_writer() && last_write.elapsed() >= WRITER_IDLE {
                    synchronizer.release_writer()?;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
    Ok(())
}

fn notify_index(context: &WorkerContext) {
    let revision = context.progress.bump_revision();
    (context.index_notifier)(IndexRevision {
        workspace_path: context.root.to_string_lossy().into_owned(),
        generation: context.generation,
        revision,
    });
}

fn collect_batch(
    initial: WorkerMessage,
    receiver: &mpsc::Receiver<WorkerMessage>,
) -> (HashSet<PathBuf>, bool) {
    let deadline = Instant::now() + EVENT_BATCH_WINDOW;
    let mut quiet_deadline = Instant::now() + EVENT_QUIET_WINDOW;
    let (initial, mut rescan) = match initial {
        WorkerMessage::Paths(paths) => (paths, false),
        WorkerMessage::Rescan(paths) => (paths, true),
    };
    let mut paths = initial.into_iter().collect::<HashSet<_>>();
    loop {
        // Flush after a quiet gap, but never extend the 80 ms absolute cap.
        let Some(remaining) = deadline.min(quiet_deadline).checked_duration_since(Instant::now()) else {
            return (paths, rescan);
        };
        match receiver.recv_timeout(remaining) {
            Ok(WorkerMessage::Paths(next)) => {
                paths.extend(next);
                quiet_deadline = Instant::now() + EVENT_QUIET_WINDOW;
            }
            Ok(WorkerMessage::Rescan(next)) => {
                paths.extend(next);
                rescan = true;
                quiet_deadline = Instant::now() + EVENT_QUIET_WINDOW;
            }
            Err(_) => return (paths, rescan),
        }
    }
}

fn enqueue(sender: &mpsc::SyncSender<WorkerMessage>, dirty: &AtomicBool, message: WorkerMessage) {
    if sender.try_send(message).is_err() {
        dirty.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[path = "worker_idle_tests.rs"]
mod idle_tests;

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "без сообщения".to_owned())
}


#[cfg(test)]
mod batch_tests {
    use super::{collect_batch, WorkerMessage, EVENT_BATCH_WINDOW};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Instant;

    fn paths(first: usize, count: usize) -> Vec<PathBuf> {
        (first..first + count)
            .map(|n| PathBuf::from(format!("/vault/Note-{n:03}.md")))
            .collect()
    }

    #[test]
    fn watcher_batch_deduplicates_paths_and_retains_rescan_request() {
        let (sender, receiver) = mpsc::channel();
        sender.send(WorkerMessage::Paths(paths(1, 4))).unwrap();
        sender.send(WorkerMessage::Rescan(paths(2, 4))).unwrap();
        let (batched, rescan) = collect_batch(WorkerMessage::Paths(paths(0, 4)), &receiver);
        assert!(rescan, "rescan notification must not be lost");
        assert_eq!(batched.len(), 6, "deduplicate events by absolute path");
        for path in paths(0, 6) {
            assert!(batched.contains(&path));
        }
    }

    #[test]
    fn watcher_batch_without_rescan_deduplicates_repeated_notifications() {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..8 {
            sender.send(WorkerMessage::Paths(paths(0, 4))).unwrap();
        }
        let (batched, rescan) = collect_batch(WorkerMessage::Paths(paths(0, 4)), &receiver);
        assert!(!rescan);
        assert_eq!(batched.len(), 4);
    }

    /// Measures event-collection overhead only. Does not invoke Tantivy or
    /// measure live desktop filesystem-event arrival distributions.
    #[test]
    #[ignore = "run via watcher event batch performance workflow"]
    fn benchmark_event_batch_window() {
        fn measure(profile: &str, initial: usize, extra_messages: usize, each: usize) {
            let mut timings = Vec::new();
            for _ in 0..3 {
                let (sender, receiver) = mpsc::channel();
                for i in 0..extra_messages {
                    sender
                        .send(WorkerMessage::Paths(paths(i * each, each)))
                        .unwrap();
                }
                let started = Instant::now();
                let (batched, rescan) =
                    collect_batch(WorkerMessage::Paths(paths(0, initial)), &receiver);
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                let expected = initial.max(extra_messages * each);
                assert_eq!(batched.len(), expected);
                assert!(!rescan);
                timings.push(ms);
            }
            timings.sort_by(f64::total_cmp);
            println!(
                "EVENT_BATCH profile={profile} initial={initial} enqueued={extra_messages} unique={} configured_window_ms={} median_collect_ms={:.3}",
                initial.max(extra_messages * each),
                EVENT_BATCH_WINDOW.as_millis(),
                timings[1]
            );
        }

        measure("one_note", 1, 0, 0);
        measure("single_message_64_notes", 64, 0, 0);
        measure("queued_8_messages_of_8_notes", 8, 8, 8);
    }
}
