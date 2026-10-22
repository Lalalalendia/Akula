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


/// Research-only: compare the unchanged SQLite metadata writer under its current
/// default page cache and a bounded 16 MiB page cache. All SQL, parsing, schema,
/// durability settings, and 10k-document transaction boundaries stay identical.
/// Invoke explicitly using AQUILUM_CACHE_BENCH_MODE=default|cache16 and
/// AQUILUM_CACHE_BENCH_SIZE=1000|10000|50000.
#[test]
#[ignore = "research: same-runner SQLite page cache ABBA comparison"]
fn benchmark_sqlite_cache_scan() {
    use std::path::PathBuf;
    let documents = std::env::var("AQUILUM_CACHE_BENCH_SIZE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000);
    assert!([1000, 10000, 50000].contains(&documents));
    let mode = std::env::var("AQUILUM_CACHE_BENCH_MODE")
        .unwrap_or_else(|_| "default".to_owned());
    assert!(mode == "default" || mode == "cache16");

    let root = Path::new("/vault");
    let mut body = String::new();
    for index in 0..32 {
        body.push_str(&format!(
            "Property{index:02}:: value-{index}\n- [ ] Task {index}\n- [[Note {index:02}]]\n"
        ));
    }
    assert_eq!(fields::inline::indexed_fields(&body).len(), 32);
    assert_eq!(wiki::parser::extract(&body).len(), 32);

    let directory = tempfile::tempdir().unwrap();
    let sqlite_path = directory.path().join("documents.sqlite3");
    let mut connection = crate::search::metadata::open(&sqlite_path).unwrap();
    if mode == "cache16" {
        connection.execute_batch("PRAGMA cache_size=-16384").unwrap();
    }
    let configured_cache_kib: i64 = connection
        .query_row("PRAGMA cache_size", [], |row| row.get(0))
        .unwrap();

    // The measured interval mirrors production's metadata work and 10k-note
    // commit grouping. Full data validation runs only after this interval.
    let body_hash = blake3::hash(body.as_bytes());
    let started = Instant::now();
    let mut sqlite_commit_ms = 0.0;
    for start in (0..documents).step_by(10_000) {
        let transaction = connection.transaction().unwrap();
        for index in start..(start + 10_000).min(documents) {
            let path = PathBuf::from(format!("/vault/Note-{index:05}.md"));
            wiki::index_document(&transaction, root, &path, &body).unwrap();
            fields::index_document(&transaction, &path, &body).unwrap();
            tasks::index_document(&transaction, &path, &body).unwrap();
            transaction
                .execute(
                    "INSERT INTO documents(path, modified_ns, size, content_hash, analyzer_version, created_ns, created_source)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        path.to_string_lossy().as_ref(),
                        index as i64,
                        body.len() as i64,
                        &body_hash.as_bytes()[..],
                        crate::search::ANALYZER_VERSION as i64,
                        index as i64,
                        0i64,
                    ],
                )
                .unwrap();
        }
        let commit_started = Instant::now();
        transaction.commit().unwrap();
        sqlite_commit_ms += commit_started.elapsed().as_secs_f64() * 1000.0;
    }
    crate::search::metadata::maintain(&connection).unwrap();
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;

    let checks = [
        ("wiki_documents", documents),
        ("wiki_links", documents * 32),
        ("note_fields", documents * 32),
        ("note_tasks", documents * 32),
        ("documents", documents),
    ];
    for (table, expected) in checks {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap();
        assert_eq!(count as usize, expected, "wrong count in {table}");
    }
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");

    // Hash every stored column, every row, in primary-key order. Ensure that
    // changing only the cache size leaves the persisted SQLite contents equal.
    let digest = cache_benchmark_digest(&connection);
    let database_bytes = std::fs::metadata(&sqlite_path).unwrap().len();
    let wal_path = PathBuf::from(format!("{}-wal", sqlite_path.to_string_lossy()));
    let wal_bytes = std::fs::metadata(&wal_path).map_or(0, |m| m.len());
    println!(
        "BENCH-CACHE documents={documents} mode={mode} configured_cache_kib={configured_cache_kib} \
         wall_ms={wall_ms:.3} sqlite_commit_ms={sqlite_commit_ms:.3} \
         digest={digest} db_bytes={database_bytes} wal_bytes={wal_bytes} integrity=ok"
    );
}

/// Streaming BLAKE3 of all stored rows, including SQL type, NULL/empty, and
/// column lengths. Query order is explicit, so both modes are comparable.
fn cache_benchmark_digest(connection: &Connection) -> String {
    use rusqlite::types::ValueRef;
    let mut hasher = blake3::Hasher::new();
    const QUERIES: &[&str] = &[
        "SELECT path, relative_key, title_key FROM wiki_documents ORDER BY path",
        "SELECT source_path, target_key, target_kind, byte_offset, offset_utf16 FROM wiki_links ORDER BY source_path, byte_offset",
        "SELECT path, ordinal, key, key_lower, kind, text, text_lower, items FROM note_fields ORDER BY path, ordinal",
        "SELECT path, ordinal, line, done, text FROM note_tasks ORDER BY path, ordinal",
        "SELECT path, modified_ns, size, content_hash, analyzer_version, created_ns, created_source FROM documents ORDER BY path",
    ];
    for query in QUERIES {
        hasher.update(query.as_bytes());
        let mut statement = connection.prepare(query).unwrap();
        let columns = statement.column_count();
        let mut cursor = statement.query([]).unwrap();
        while let Some(row) = cursor.next().unwrap() {
            for column in 0..columns {
                match row.get_ref(column).unwrap() {
                    ValueRef::Null => { hasher.update(&[0]); }
                    ValueRef::Integer(value) => {
                        hasher.update(&[1]);
                        hasher.update(&value.to_le_bytes());
                    }
                    ValueRef::Real(value) => {
                        hasher.update(&[2]);
                        hasher.update(&value.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(bytes) => {
                        hasher.update(&[3]);
                        hasher.update(&(bytes.len() as u64).to_le_bytes());
                        hasher.update(bytes);
                    }
                    ValueRef::Blob(bytes) => {
                        hasher.update(&[4]);
                        hasher.update(&(bytes.len() as u64).to_le_bytes());
                        hasher.update(bytes);
                    }
                }
            }
        }
    }
    hasher.finalize().to_hex().to_string()
}
