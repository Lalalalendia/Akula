//! Diagnostic-only per-scan timing. No global counters shared across scans:
 //! the existing synchronous IndexSynchronizer owns the collection thread.
 //! Read/hash CPU time comes from prepared files produced on worker threads.
use std::cell::RefCell;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, Ordering};

static PREPARED: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static READ_NS: AtomicU64 = AtomicU64::new(0);
static HASH_NS: AtomicU64 = AtomicU64::new(0);

#[repr(usize)]
#[derive(Clone, Copy)]
pub(crate) enum Stage {
    MetadataLoad,
    DirectoryNext,
    Probe,
    Writer,
    PrepareWall,
    Created,
    TantivyAdd,
    Wiki,
    Fields,
    Tasks,
    MetadataUpsert,
    TantivyCommit,
    ReaderReload,
    SqliteCommit,
    Prune,
    Maintenance,
}

const LABELS: [&str; 16] = [
    "metadata_load",
    "directory_next",
    "probe",
    "writer",
    "prepare_wall",
    "created",
    "tantivy_add",
    "wiki",
    "fields",
    "tasks",
    "metadata_upsert",
    "tantivy_commit",
    "reader_reload",
    "sqlite_commit",
    "prune",
    "maintenance",
];

struct Metrics {
    ns: [u128; LABELS.len()],
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            ns: [0; LABELS.len()],
        }
    }
}

thread_local! {
    static METRICS: RefCell<Metrics> = RefCell::new(Metrics::default());
}

pub(crate) fn reset() {
    METRICS.with(|cell| *cell.borrow_mut() = Metrics::default());
    PREPARED.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    READ_NS.store(0, Ordering::Relaxed);
    HASH_NS.store(0, Ordering::Relaxed);
}

pub(crate) fn add_ns(stage: Stage, nanoseconds: u128) {
    METRICS.with(|cell| {
        let mut data = cell.borrow_mut();
        data.ns[stage as usize] = data.ns[stage as usize].saturating_add(nanoseconds);
    });
}

pub(crate) fn time<T>(stage: Stage, operation: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = operation();
    add_ns(stage, start.elapsed().as_nanos());
    result
}

pub(crate) fn prepared(bytes: usize, read_ns: u128, hash_ns: u128) {
    PREPARED.fetch_add(1, Ordering::Relaxed);
    BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    READ_NS.fetch_add(read_ns.min(u64::MAX as u128) as u64, Ordering::Relaxed);
    HASH_NS.fetch_add(hash_ns.min(u64::MAX as u128) as u64, Ordering::Relaxed);
}

/// Times the actual recursive filesystem iterator's next(), including the
/// filters applied before the result reaches full_scan.
pub(crate) struct TimedIterator<I>(I);

impl<I: Iterator> TimedIterator<I> {
    pub(crate) fn new(inner: I) -> Self {
        Self(inner)
    }
}

impl<I: Iterator> Iterator for TimedIterator<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        time(Stage::DirectoryNext, || self.0.next())
    }
}

/// Per-scan wall and stage timings. Timers other than ReadCpu/HashCpu are on
/// the coordinator thread. ReadCpu/HashCpu are sums across parallel workers:
/// they overlap PrepareWall, so columns MUST NOT be added into a grand total.
/// This probe is enabled only by an explicit Cargo feature in CI.
pub(crate) fn print_scan(documents: u64, wall: Duration) {
    METRICS.with(|cell| {
        let data = cell.borrow();
        print!(
            "INDEX-STAGE documents={documents} prepared={} input_bytes={} wall_ms={:.3}",
            PREPARED.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
            wall.as_secs_f64() * 1000.0,
        );
        for (label, nanoseconds) in LABELS.iter().zip(data.ns.iter()) {
            print!(" {label}_ms={:.3}", *nanoseconds as f64 / 1_000_000.0);
        }
        println!(
            " read_cpu_sum_ms={:.3} hash_cpu_sum_ms={:.3}",
            READ_NS.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            HASH_NS.load(Ordering::Relaxed) as f64 / 1_000_000.0,
        );
    });
}
