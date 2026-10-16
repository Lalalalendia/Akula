//! Reproducible, opt-in benchmark corpus for the Argon performance experiment.
//! These tests intentionally measure the real production paths without modifying them.
//! Run: cargo test --locked --release -p aquilum-core bench_ -- --ignored --nocapture --test-threads=1

use super::{fields, index::SearchIndex, progress::IndexProgress, sync::IndexSynchronizer};
use crate::documents::merge::merge::merge_external_change;
use std::fmt::Write as _;
use std::hint::black_box;
use std::fs;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;

fn report(name: &str, mut samples_ns: Vec<u128>) {
    samples_ns.sort_unstable();
    assert!(!samples_ns.is_empty());
    println!(
        "PERF_RESULT name={name} min_ns={} median_ns={} max_ns={} samples_ns={samples_ns:?}",
        samples_ns[0],
        samples_ns[samples_ns.len() / 2],
        samples_ns[samples_ns.len() - 1],
    );
}

fn repeated(calls: usize, mut execute: impl FnMut()) -> Vec<u128> {
    // Warm-up is excluded from timing.
    for _ in 0..3 {
        execute();
    }
    let mut samples = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for _ in 0..calls {
            execute();
        }
        samples.push(started.elapsed().as_nanos() / calls as u128);
    }
    samples
}

#[test]
#[ignore = "run explicitly in the performance workflow"]
fn bench_inline_fields() {
    let mut text = String::new();
    for i in 0..8_000 {
        if i % 40 == 0 {
            writeln!(text, "- Category:: ref_{i}").unwrap();
        } else {
            writeln!(text, "A normal Markdown paragraph {i} with prose and a [[WikiLink]] but no metadata.").unwrap();
        }
    }
    let found = fields::inline::indexed_fields(&text);
    assert_eq!(found.len(), 1);
    report("inline_fields", repeated(60, || {
        black_box(fields::inline::indexed_fields(black_box(&text)));
    }));
}

#[test]
#[ignore = "run explicitly in the performance workflow"]
fn bench_merge_nonoverlap() {
    let mut base = String::new();
    for i in 0..420 {
        writeln!(base, "Section {i}: multiple paragraphs and long repeated metadata values for line comparison.").unwrap();
    }
    let disk = base.replace("Section 120:", "Section 120: EXTERNAL");
    let current = base.replace("Section 300:", "Section 300: LOCAL");
    assert!(!merge_external_change(&base, &disk, &current).edits.is_empty());
    report("merge_nonoverlap", repeated(10, || {
        black_box(merge_external_change(black_box(&base), black_box(&disk), black_box(&current)));
    }));
}

#[test]
#[ignore = "run explicitly in the performance workflow"]
fn bench_full_scan_cold() {
    // Deterministic medium vault: frontmatter, wiki links, fields and tasks.
    let scratch = tempfile::tempdir().unwrap();
    let vault = scratch.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    const DOCUMENTS: usize = 512;
    for i in 0..DOCUMENTS {
        let mut note = format!("---\ntags: [rust, benchmark]\npriority: {}\n---\n# Note {i}\n", i % 5);
        for line in 0..32 {
            writeln!(
                note,
                "- [ ] Task {line} for note {i}; [Score:: {}] [[Note-{:04}]] with detailed research background and links.",
                line % 10,
                (i + line) % DOCUMENTS
            ).unwrap();
        }
        fs::write(vault.join(format!("Note-{i:04}.md")), note).unwrap();
    }

    let mut samples = Vec::new();
    for trial in 0..3 {
        let storage = scratch.path().join(format!("trial-{trial}"));
        fs::create_dir_all(&storage).unwrap();
        let index = Arc::new(SearchIndex::open(&storage.join("tantivy")).unwrap());
        let progress = Arc::new(IndexProgress::new(0, 1));
        let mut sync = IndexSynchronizer::new(
            vault.clone(),
            Arc::clone(&index),
            &storage.join("documents.sqlite3"),
            progress,
        ).unwrap();
        let start = Instant::now();
        sync.full_scan(&AtomicBool::new(false)).unwrap();
        let elapsed = start.elapsed().as_nanos();
        index.reload().unwrap();
        assert_eq!(index.document_count(), DOCUMENTS as u64);
        samples.push(elapsed);
    }
    report("full_scan_cold_512", samples);
}
