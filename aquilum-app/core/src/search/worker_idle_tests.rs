use super::{collect_batch, WorkerMessage, EVENT_BATCH_WINDOW, EVENT_QUIET_WINDOW};
use crate::search::benches::support::{open_bench_index, write_note};
use crate::search::query::Request;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn paths(first: usize, count: usize) -> Vec<PathBuf> {
    (first..first + count)
        .map(|n| PathBuf::from(format!("/vault/Note-{n:03}.md")))
        .collect()
}

// Historical fixed-window implementation, compiled only for same-runner
// measurement against the candidate quiet-period batching policy.
fn baseline_collect_batch(
    initial: WorkerMessage,
    receiver: &mpsc::Receiver<WorkerMessage>,
) -> (HashSet<PathBuf>, bool) {
    let deadline = Instant::now() + EVENT_BATCH_WINDOW;
    let (initial, mut rescan) = match initial {
        WorkerMessage::Paths(paths) => (paths, false),
        WorkerMessage::Rescan(paths) => (paths, true),
    };
    let mut paths = initial.into_iter().collect::<HashSet<_>>();
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return (paths, rescan);
        };
        match receiver.recv_timeout(remaining) {
            Ok(WorkerMessage::Paths(next)) => paths.extend(next),
            Ok(WorkerMessage::Rescan(next)) => {
                paths.extend(next);
                rescan = true;
            }
            Err(_) => return (paths, rescan),
        }
    }
}

#[test]
fn quiet_window_retains_rescan_and_unique_file_paths() {
    let (send, recv) = mpsc::channel();
    send.send(WorkerMessage::Paths(paths(1, 5))).unwrap();
    send.send(WorkerMessage::Rescan(paths(3, 3))).unwrap();
    let (unique, rescan) = collect_batch(WorkerMessage::Paths(paths(0, 4)), &recv);
    assert!(rescan);
    assert_eq!(unique.len(), 6);
    for path in paths(0, 6) {
        assert!(unique.contains(&path));
    }
}

#[test]
fn quiet_window_is_strictly_smaller_than_batch_maximum() {
    assert!(EVENT_QUIET_WINDOW < EVENT_BATCH_WINDOW);
    assert_eq!(EVENT_QUIET_WINDOW, Duration::from_millis(40));
    assert_eq!(EVENT_BATCH_WINDOW, Duration::from_millis(80));
}

fn measure_queue(
    algorithm: fn(WorkerMessage, &mpsc::Receiver<WorkerMessage>) -> (HashSet<PathBuf>, bool),
    first: usize,
    queued_messages: usize,
    each: usize,
) -> (f64, usize) {
    let (send, recv) = mpsc::channel();
    for i in 0..queued_messages {
        send.send(WorkerMessage::Paths(paths(i * each, each))).unwrap();
    }
    let started = Instant::now();
    let (result, rescan) = algorithm(WorkerMessage::Paths(paths(0, first)), &recv);
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    assert!(!rescan);
    assert_eq!(result.len(), first.max(queued_messages * each));
    (elapsed, result.len())
}

#[test]
#[ignore = "same-runner idle-window comparison"]
fn benchmark_quiet_window_vs_fixed_80ms() {
    for (name, first, messages, each) in [
        ("single", 1, 0, 0),
        ("one_message_64", 64, 0, 0),
        ("queued_8x8", 8, 8, 8),
    ] {
        let mut legacy = Vec::new();
        let mut candidate = Vec::new();
        for baseline_first in [true, false, false, true] {
            if baseline_first {
                legacy.push(measure_queue(baseline_collect_batch, first, messages, each).0);
                candidate.push(measure_queue(collect_batch, first, messages, each).0);
            } else {
                candidate.push(measure_queue(collect_batch, first, messages, each).0);
                legacy.push(measure_queue(baseline_collect_batch, first, messages, each).0);
            }
        }
        legacy.sort_by(f64::total_cmp);
        candidate.sort_by(f64::total_cmp);
        println!(
            "QUIET_WINDOW profile={name} max_ms={} idle_ms={} baseline_median_ms={:.3} candidate_median_ms={:.3} saved_ms={:.3}",
            EVENT_BATCH_WINDOW.as_millis(),
            EVENT_QUIET_WINDOW.as_millis(),
            legacy[2],
            candidate[2],
            legacy[2] - candidate[2],
        );
    }
}

#[test]
#[ignore = "same-runner end-to-end synthetic save latency"]
fn benchmark_quiet_window_until_search_visible() {
    let mut bench = open_bench_index(512);
    bench.synchronizer.full_scan(&bench.cancelled).unwrap();
    bench.synchronizer.release_writer().unwrap();
    let saved = bench.vault.join("Note 7.md");
    let mut values = [Vec::new(), Vec::new()];
    for (run, use_idle) in [false, true, true, false].into_iter().enumerate() {
        for sample in 0..5 {
            write_note(&bench.vault, 7, 54 + run * 5 + sample);
            let marker = format!("quietwindowmarker{run:02}{sample:02}");
            writeln!(OpenOptions::new().append(true).open(&saved).unwrap(), "{marker}").unwrap();
            let (send, recv) = mpsc::channel();
            let started = Instant::now();
            let (batched, rescan) = if use_idle {
                collect_batch(WorkerMessage::Paths(vec![saved.clone()]), &recv)
            } else {
                baseline_collect_batch(WorkerMessage::Paths(vec![saved.clone()]), &recv)
            };
            assert!(!rescan);
            assert!(bench.synchronizer.apply_paths(batched).unwrap());
            assert_eq!(bench.search_index.document_count(), 512);
            let result = bench.search_index.search(&Request::new(&marker)).unwrap();
            assert!(result.results.iter().any(|hit| hit.path.ends_with("Note 7.md")));
            if sample > 0 {
                values[usize::from(use_idle)].push(started.elapsed().as_secs_f64() * 1_000.0);
            }
            drop(send);
        }
    }
    for row in &mut values {
        row.sort_by(f64::total_cmp);
    }
    println!(
        "QUIET_SEARCH baseline_median_ms={:.3} candidate_median_ms={:.3} saved_ms={:.3} samples_each={} visible_after_commit=true",
        values[0][values[0].len()/2], values[1][values[1].len()/2],
        values[0][values[0].len()/2] - values[1][values[1].len()/2],
        values[0].len(),
    );
}


/// A synthetic event stream, not field telemetry: show how often 20 ms
/// quiet gaps cause additional commits relative to the fixed 80 ms window.
#[test]
#[ignore = "same-runner event-spacing tradeoff comparison"]
fn benchmark_quiet_window_stream_patterns() {
    use std::thread;

    fn profile(
        algorithm: fn(WorkerMessage, &mpsc::Receiver<WorkerMessage>) -> (HashSet<PathBuf>, bool),
        gap: Duration,
    ) -> (usize, f64, Vec<usize>) {
        const EVENTS: usize = 8;
        let (sender, receiver) = mpsc::channel();
        // Keep the channel alive after the producer finishes: both batching
        // algorithms must flush at their own deadline, not on disconnection.
        let kept_alive = sender.clone();
        let producer = thread::spawn(move || {
            for n in 0..EVENTS {
                if n != 0 {
                    thread::sleep(gap);
                }
                sender.send(WorkerMessage::Paths(paths(n, 1))).unwrap();
            }
        });

        let start = Instant::now();
        let mut commits = 0;
        let mut received = HashSet::new();
        let mut sizes = Vec::new();
        while received.len() < EVENTS {
            let first = receiver.recv().unwrap();
            let (batch, rescan) = algorithm(first, &receiver);
            assert!(!rescan);
            sizes.push(batch.len());
            received.extend(batch);
            commits += 1;
        }
        producer.join().unwrap();
        drop(kept_alive);
        assert_eq!(received.len(), EVENTS);
        (commits, start.elapsed().as_secs_f64() * 1000.0, sizes)
    }

    for (name, gap) in [
        ("dense_5ms", Duration::from_millis(5)),
        ("moderate_15ms", Duration::from_millis(15)),
        ("spaced_30ms", Duration::from_millis(30)),
        ("spaced_45ms", Duration::from_millis(45)),
        ("spaced_60ms", Duration::from_millis(60)),
    ] {
        let (old_commits, old_ms, old_sizes) = profile(baseline_collect_batch, gap);
        let (new_commits, new_ms, new_sizes) = profile(collect_batch, gap);
        println!(
            "QUIET_STREAM profile={name} gap_ms={} events=8 fixed_commits={old_commits} quiet_commits={new_commits} fixed_total_ms={old_ms:.3} quiet_total_ms={new_ms:.3} fixed_groups={old_sizes:?} quiet_groups={new_sizes:?}",
            gap.as_millis()
        );
    }
}
