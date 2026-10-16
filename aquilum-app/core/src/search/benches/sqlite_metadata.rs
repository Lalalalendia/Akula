//! Benchmark SQLite indexing of wiki links, inline/frontmatter fields and tasks.
//! This is intentionally an ignored test, so production tests do not slow down.
use super::support::record;
use crate::search::{fields, tasks, wiki};
use rusqlite::Connection;
use std::path::Path;
use std::time::Instant;

#[test]
#[ignore = "run in Core performance baseline CI"]
fn benchmark_sqlite_metadata() {
    const DOCUMENTS: usize = 512;
    const SAMPLES: usize = 5;
    let body = "---\ntags: [rust, performance]\nkind: benchmark\n---\n# Subject\n\nOwner:: Core\nScore:: 8\n\n[[Related document]]\n- [ ] First task\n- [x] Second task\n- [ ] Third task\n";
    let root = Path::new("/vault");
    let mut samples = Vec::with_capacity(SAMPLES);

    for trial in 0..=SAMPLES {
        let mut connection = Connection::open_in_memory().unwrap();
        wiki::open_schema(&connection).unwrap();
        fields::open_schema(&connection).unwrap();
        tasks::open_schema(&connection).unwrap();
        let started = Instant::now();
        for n in 0..DOCUMENTS {
            let path = root.join(format!("Note-{n:04}.md"));
            let transaction = connection.transaction().unwrap();
            wiki::index_document(&transaction, root, &path, body).unwrap();
            fields::index_document(&transaction, &path, body).unwrap();
            tasks::index_document(&transaction, &path, body).unwrap();
            transaction.commit().unwrap();
        }
        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        for table in ["wiki_links", "note_fields", "note_tasks"] {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            let count: i64 = connection.query_row(&sql, [], |row| row.get(0)).unwrap();
            assert!(count >= DOCUMENTS as i64, "missing indexed rows in {table}");
        }
        // The first run warms allocator and CPU caches; exclude it from results.
        if trial != 0 {
            samples.push(elapsed_ms);
        }
    }
    samples.sort_by(f64::total_cmp);
    let line = format!(
        "documents={DOCUMENTS} samples={SAMPLES} min_ms={:.3} median_ms={:.3} max_ms={:.3}",
        samples[0], samples[SAMPLES / 2], samples[SAMPLES - 1],
    );
    println!("BENCH-SQLITE {line}");
    record("sqlite_metadata", &line);
}
