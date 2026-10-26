use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use super::Counts;

fn fixture(notes: usize) -> (Counts, Vec<String>, Vec<String>) {
    // Approximately 10 files per leaf and 25 leaf folders per parent.
    // This models folder count costs, NOT disk IO or complete Akula startup.
    let folders = notes / 10;
    let branches = folders.div_ceil(25);
    let mut counts = Counts::default();
    counts.own.insert("c:/vault".to_owned(), branches as u32);
    for i in 0..folders {
        let branch = format!("c:/vault/b{:05}", i / 25);
        counts.own.entry(branch.clone()).or_insert(25);
        counts.own.insert(format!("{branch}/f{i:05}"), 10);
    }
    let collapsed = counts.own.keys()
        .filter(|path| path.starts_with("c:/vault/b") && !path["c:/vault/".len()..].contains('/'))
        .cloned().collect();
    let expanded = counts.own.keys().filter(|path| *path != "c:/vault").cloned().collect();
    (counts, collapsed, expanded)
}

fn reference(counts: &Counts, shown: &[String]) -> u64 {
    shown.iter().map(|key| u64::from(counts.total(Path::new(key)).unwrap())).sum()
}

fn indexed(counts: &Counts, shown: &[String]) -> u64 {
    // Index creation is INCLUDED in every timed rebuild, as in production.
    let lookup = counts.indexed();
    shown.iter().map(|key| u64::from(lookup.total(key).unwrap())).sum()
}

fn timed<F: FnOnce() -> u64>(action: F) -> (Duration, u64) {
    let start = Instant::now();
    let result = black_box(action());
    (start.elapsed(), result)
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000_000.0
}

fn percentile(times: &[Duration], centile: usize) -> f64 {
    let mut values: Vec<Duration> = times.to_vec();
    values.sort();
    let index = (values.len() * centile).div_ceil(100).saturating_sub(1);
    micros(values[index])
}

#[test]
#[ignore = "opt-in Windows paired release diagnostic; timing is not a correctness gate"]
fn benchmark_filetree_total_lookup_abba() {
    for notes in [1_000usize, 10_000, 50_000] {
        let (counts, collapsed, expanded) = fixture(notes);
        for (shape, shown) in [("collapsed", &collapsed), ("expanded", &expanded)] {
            assert!(!shown.is_empty());
            let expected = reference(&counts, shown);
            assert_eq!(indexed(&counts, shown), expected);
            // Alternating order reduces systematic warm-cache bias.
            let (mut original, mut candidate) = (Vec::new(), Vec::new());
            for round in 0..8 {
                if round % 2 == 0 {
                    let (d, actual) = timed(|| reference(&counts, black_box(shown)));
                    assert_eq!(actual, expected);
                    original.push(d);
                    let (d, actual) = timed(|| indexed(&counts, black_box(shown)));
                    assert_eq!(actual, expected);
                    candidate.push(d);
                } else {
                    let (d, actual) = timed(|| indexed(&counts, black_box(shown)));
                    assert_eq!(actual, expected);
                    candidate.push(d);
                    let (d, actual) = timed(|| reference(&counts, black_box(shown)));
                    assert_eq!(actual, expected);
                    original.push(d);
                }
            }
            // No timing assertion: the same corpus / runner may be noisy.
            println!(
                "FILETREE_TOTAL_AB notes={notes} folders={} shown={} mode={shape} old_p50_us={:.2} new_p50_us={:.2} old_p95_us={:.2} new_p95_us={:.2} checksum={expected}",
                counts.own.len(), shown.len(),
                percentile(&original, 50), percentile(&candidate, 50),
                percentile(&original, 95), percentile(&candidate, 95),
            );
        }
    }
}
