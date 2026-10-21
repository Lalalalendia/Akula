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
    WikiParse,
    Fields,
    FieldsParse,
    Tasks,
    TasksParse,
    MetadataUpsert,
    TantivyCommit,
    TantivyPrepare,
    TantivyFinalize,
    ReaderReload,
    SqliteCommit,
    Prune,
    Maintenance,
}

const LABELS: [&str; 21] = [
    "metadata_load",
    "directory_next",
    "probe",
    "writer",
    "prepare_wall",
    "created",
    "tantivy_add",
    "wiki",
    "wiki_parse",
    "fields",
    "fields_parse",
    "tasks",
    "tasks_parse",
    "metadata_upsert",
    "tantivy_commit",
    "tantivy_prepare",
    "tantivy_finalize",
    "reader_reload",
    "sqlite_commit",
    "prune",
    "maintenance",
];

struct Metrics {
    ns: [u128; LABELS.len()],
    commits: u64,
    wiki_rows: u64,
    field_rows: u64,
    task_rows: u64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            ns: [0; LABELS.len()],
            commits: 0,
            wiki_rows: 0,
            field_rows: 0,
            task_rows: 0,
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

pub(crate) fn record_rows(stage: Stage, count: usize) {
    METRICS.with(|cell| {
        let mut data = cell.borrow_mut();
        match stage {
            Stage::Wiki => data.wiki_rows += count as u64,
            Stage::Fields => data.field_rows += count as u64,
            Stage::Tasks => data.task_rows += count as u64,
            _ => unreachable!("row counter applies only to structured projections"),
        }
    });
}

pub(crate) fn committed() {
    METRICS.with(|cell| cell.borrow_mut().commits += 1);
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

/// Per-scan wall and stage timings. Nested stages (wiki/wiki_parse,
/// fields/fields_parse, tasks/tasks_parse and commit/prepare/finalize/reload)
/// overlap their parent and must not be added into a grand total.
/// ReadCpu/HashCpu are sums across parallel workers, not CPU clock samples.
/// This probe is enabled only by an explicit Cargo feature in CI.
pub(crate) fn print_scan(documents: u64, wall: Duration) {
    METRICS.with(|cell| {
        let data = cell.borrow();
        print!(
            "INDEX-STAGE documents={documents} prepared={} input_bytes={} wall_ms={:.3} commits={} wiki_rows={} field_rows={} task_rows={}",
            PREPARED.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
            wall.as_secs_f64() * 1000.0,
            data.commits,
            data.wiki_rows,
            data.field_rows,
            data.task_rows,
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
