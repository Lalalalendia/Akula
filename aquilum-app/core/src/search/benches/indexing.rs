use super::support::{directory_size, open_bench_index, record, write_note};
use std::fs;
use std::time::Instant;

#[test]
#[ignore = "run with AQUILUM_BENCH_SIZES=1000,10000,100000 cargo test --release benchmark_indexing -- --ignored --nocapture"]
fn benchmark_indexing() {
    let sizes = std::env::var("AQUILUM_BENCH_SIZES").unwrap_or_else(|_| "1000".to_owned());
    for count in sizes.split(',').filter_map(|value| value.trim().parse::<usize>().ok()) {
        run_case(count);
    }
}

fn run_case(document_count: usize) {
    let mut bench = open_bench_index(document_count);

    let started = Instant::now();
    bench.synchronizer.full_scan(&bench.cancelled).unwrap();
    let cold_ms = started.elapsed().as_millis();

    let started = Instant::now();
    bench.synchronizer.full_scan(&bench.cancelled).unwrap();
    let warm_ms = started.elapsed().as_millis();

    let touched = (document_count / 100).clamp(1, document_count);
    for index in 0..touched {
        write_note(&bench.vault, index, 55);
    }
    let started = Instant::now();
    bench.synchronizer.full_scan(&bench.cancelled).unwrap();
    let incremental_ms = started.elapsed().as_millis();

    for index in 0..touched {
        fs::remove_file(bench.vault.join(format!("Note {index}.md"))).unwrap();
    }
    let started = Instant::now();
    bench.synchronizer.full_scan(&bench.cancelled).unwrap();
    let deletion_ms = started.elapsed().as_millis();

    let saved = bench.vault.join(format!("Note {touched}.md"));
    let mut saves = (0..5)
        .map(|round| {
            write_note(&bench.vault, touched, 56 + round);
            let started = Instant::now();
            bench
                .synchronizer
                .apply_paths(std::collections::HashSet::from([saved.clone()]))
                .unwrap();
            started.elapsed().as_secs_f64() * 1000.0
        })
        .collect::<Vec<_>>();
    saves.sort_by(f64::total_cmp);
    let (save_min_ms, save_median_ms) = (saves[0], saves[saves.len() / 2]);
    bench.synchronizer.release_writer().unwrap();
    write_note(&bench.vault, touched, 62);
    let started = Instant::now();
    bench
        .synchronizer
        .apply_paths(std::collections::HashSet::from([saved.clone()]))
        .unwrap();
    let save_after_idle_ms = started.elapsed().as_secs_f64() * 1000.0;

    let index_bytes = directory_size(&bench.directory.path().join("index"));
    let docs_per_sec_cold = document_count as f64 / (cold_ms.max(1) as f64 / 1000.0);

    let line = format!(
        "documents={document_count} cold_ms={cold_ms} warm_ms={warm_ms} incremental_touched={touched} incremental_ms={incremental_ms} deletion_ms={deletion_ms} save_min_ms={save_min_ms:.1} save_median_ms={save_median_ms:.1} save_after_idle_ms={save_after_idle_ms:.1} docs_per_sec_cold={docs_per_sec_cold:.0} index_bytes={index_bytes}"
    );
    println!("BENCH-INDEX {line}");
    record("indexing", &line);
}

#[test]
#[ignore = "run with cargo test --release benchmark_save_breakdown -- --ignored --nocapture"]
fn benchmark_save_breakdown() {
    let mut bench = open_bench_index(1000);
    bench.synchronizer.full_scan(&bench.cancelled).unwrap();
    bench.synchronizer.release_writer().unwrap();
    let mut writer = bench
        .search_index
        .index
        .writer_with_num_threads(
            crate::search::sync::INDEX_THREADS,
            crate::search::sync::INDEX_MEMORY_BUDGET,
        )
        .unwrap();
    let mut connection = crate::search::metadata::open(&bench.metadata).unwrap();
    let saved = bench.vault.join("Note 7.md");
    let mut steps = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut prepare_steps = Vec::new();
    let mut finalize_steps = Vec::new();
    for round in 0..7 {
        write_note(&bench.vault, 7, 60 + round);
        let transaction = connection.transaction().unwrap();
        let started = Instant::now();
        crate::search::index_document::apply_path(&bench.vault, &bench.search_index, &writer, &transaction, &saved).unwrap();
        steps[0].push(started.elapsed());
        // Equivalent to IndexWriter::commit(), split into the two API
        // phases to identify segment flush vs durable commit publication.
        let started = Instant::now();
        let prepared = writer.prepare_commit().unwrap();
        let prepare_elapsed = started.elapsed();
        let started = Instant::now();
        prepared.commit().unwrap();
        let finalize_elapsed = started.elapsed();
        steps[1].push(prepare_elapsed + finalize_elapsed);
        prepare_steps.push(prepare_elapsed);
        finalize_steps.push(finalize_elapsed);
        let started = Instant::now();
        bench.search_index.reload().unwrap();
        steps[2].push(started.elapsed());
        let started = Instant::now();
        transaction.commit().unwrap();
        steps[3].push(started.elapsed());
        let started = Instant::now();
        crate::search::metadata::maintain(&connection).unwrap();
        steps[4].push(started.elapsed());
    }
    let fsync = (0..7)
        .map(|round| {
            let path = bench.directory.path().join(format!("fsync-{round}.bin"));
            let mut file = fs::File::create(&path).unwrap();
            std::io::Write::write_all(&mut file, &[7u8; 4096]).unwrap();
            let started = Instant::now();
            file.sync_all().unwrap();
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let median = |values: &[std::time::Duration]| {
        let mut sorted = values.to_vec();
        sorted.sort();
        sorted[sorted.len() / 2].as_secs_f64() * 1000.0
    };
    let line = format!(
        "apply_ms={:.2} tantivy_commit_ms={:.2} tantivy_prepare_ms={:.2} tantivy_finalize_ms={:.2} reload_ms={:.2} sqlite_commit_ms={:.2} maintain_ms={:.2} fsync_4k_ms={:.2}",
        median(&steps[0]),
        median(&steps[1]),
        median(&prepare_steps),
        median(&finalize_steps),
        median(&steps[2]),
        median(&steps[3]),
        median(&steps[4]),
        median(&fsync),
    );
    println!("BENCH-SAVE {line}");
    record("save_breakdown", &line);
}



/// Measure the cost of Tantivy commits when independent file notifications
/// are already grouped into one apply_paths call. This does NOT change the
/// production 80 ms watcher window or defer search index freshness.
#[test]
#[ignore = "run via tantivy commit batch performance workflow"]
fn benchmark_commit_batch_scaling() {
    use crate::search::index::SearchIndex;
    use crate::search::query::Request;
    use std::collections::HashSet;
    use std::fs::OpenOptions;
    use std::io::Write;

    const UPDATED: usize = 64;
    const DOCUMENTS: usize = 512;

    fn measure(group_size: usize, order: usize) -> (f64, f64, f64) {
        let mut bench = open_bench_index(DOCUMENTS);
        bench.synchronizer.full_scan(&bench.cancelled).unwrap();
        bench.synchronizer.release_writer().unwrap();
        assert_eq!(bench.search_index.document_count(), DOCUMENTS as u64);

        let mut total_ms = 0.0;
        let mut commit_times = Vec::new();
        for start in (0..UPDATED).step_by(group_size) {
            let mut paths = HashSet::new();
            for note in start..(start + group_size).min(UPDATED) {
                write_note(&bench.vault, note, 55);
                let path = bench.vault.join(format!("Note {note}.md"));
                let marker = format!("freshnesscode{order}note{note}");
                writeln!(OpenOptions::new().append(true).open(&path).unwrap(), "{marker}").unwrap();
                paths.insert(path);
            }
            let started = Instant::now();
            assert!(bench.synchronizer.apply_paths(paths).unwrap());
            let millis = started.elapsed().as_secs_f64() * 1_000.0;
            total_ms += millis;
            commit_times.push(millis);

            // The latest saved note MUST be findable directly after the
            // synchronizer returns, without waiting for a later event.
            let last_note = (start + group_size).min(UPDATED) - 1;
            let marker = format!("freshnesscode{order}note{last_note}");
            let result = bench.search_index.search(&Request::new(&marker)).unwrap();
            assert!(
                result.results.iter().any(|r| r.path.ends_with(&format!("Note {last_note}.md"))),
                "saved note {last_note} must be visible after commit"
            );
            assert_eq!(bench.search_index.document_count(), DOCUMENTS as u64);
        }
        bench.synchronizer.release_writer().unwrap();
        let reopened = SearchIndex::open(&bench.directory.path().join("index")).unwrap();
        assert_eq!(reopened.document_count(), DOCUMENTS as u64);
        for note in 0..UPDATED {
            let marker = format!("freshnesscode{order}note{note}");
            let result = reopened.search(&Request::new(&marker)).unwrap();
            assert!(
                result.results.iter().any(|r| r.path.ends_with(&format!("Note {note}.md"))),
                "saved note {note} must survive reopening the persisted index"
            );
        }
        commit_times.sort_by(f64::total_cmp);
        let middle = commit_times[commit_times.len() / 2];
        let highest = *commit_times.last().unwrap();
        println!(
            "TANTIVY_BATCH group_size={group_size} order={order} notes={UPDATED} commits={} total_ms={total_ms:.3} per_note_ms={:.3} median_call_ms={middle:.3} worst_call_ms={highest:.3} durable=true immediate_search=true",
            commit_times.len(), total_ms / UPDATED as f64
        );
        (total_ms, middle, highest)
    }

    // ABCCBA order on one runner, using the exact production indexing path.
    // Each test starts from the same deterministic 512-note vault.
    for (order, group) in [1, 8, 64, 64, 8, 1].into_iter().enumerate() {
        measure(group, order);
    }
}
