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


/// Runs only when explicitly selected by the research workflow.
#[test]
#[ignore = "research: compare SQLite field insertion modes"]
fn benchmark_fields_sqlite_batch() {
    use std::path::PathBuf;
    let documents = std::env::var("AQUILUM_FIELD_BENCH_SIZE")
        .ok()
        .and_then(|text| text.parse::<usize>().ok())
        .unwrap_or(1000);
    assert!([1000, 10000, 50000].contains(&documents));
    let mode = std::env::var("AQUILUM_FIELD_BENCH_MODE")
        .unwrap_or_else(|_| "baseline".to_owned());
    assert!(mode == "baseline" || mode == "batch");
    let mut body = String::new();
    for number in 0..32 {
        body.push_str(&format!("Property{number:02}:: Значение {number:02} Straße 中文\n"));
    }
    assert_eq!(fields::inline::indexed_fields(&body).len(), 32);
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("fields.sqlite3");
    let mut connection = crate::search::metadata::open(&db).unwrap();
    let started = Instant::now();
    for begin in (0..documents).step_by(10000) {
        let transaction = connection.transaction().unwrap();
        for n in begin..(begin + 10000).min(documents) {
            let path = PathBuf::from(format!("/vault/Note-{n:05}.md"));
            if mode == "baseline" {
                fields::index_document(&transaction, &path, &body).unwrap();
            } else {
                fields::repository::index_document_batched(&transaction, &path, &body).unwrap();
            }
        }
        transaction.commit().unwrap();
    }
    crate::search::metadata::maintain(&connection).unwrap();
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM note_fields", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, (documents * 32) as i64);
    let digest = fields_digest(&connection);
    println!("BENCH-FIELDS documents={documents} mode={mode} rows={count} wall_ms={wall_ms:.3} digest={digest}");
}

fn fields_digest(connection: &Connection) -> String {
    use rusqlite::types::ValueRef;
    let mut statement = connection
        .prepare("SELECT path, ordinal, key, key_lower, kind, text, text_lower, items FROM note_fields ORDER BY path, ordinal")
        .unwrap();
    let mut rows = statement.query([]).unwrap();
    let mut hash = blake3::Hasher::new();
    while let Some(row) = rows.next().unwrap() {
        for column in 0..8 {
            match row.get_ref(column).unwrap() {
                ValueRef::Null => { hash.update(&[0]); }
                ValueRef::Integer(value) => {
                    hash.update(&[1]);
                    hash.update(&value.to_le_bytes());
                }
                ValueRef::Real(value) => {
                    hash.update(&[2]);
                    hash.update(&value.to_bits().to_le_bytes());
                }
                ValueRef::Text(bytes) => {
                    hash.update(&[3]);
                    hash.update(&(bytes.len() as u64).to_le_bytes());
                    hash.update(bytes);
                }
                ValueRef::Blob(bytes) => {
                    hash.update(&[4]);
                    hash.update(&(bytes.len() as u64).to_le_bytes());
                    hash.update(bytes);
                }
            }
        }
    }
    hash.finalize().to_hex().to_string()
}
