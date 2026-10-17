// Standalone A/B benchmark. Workflow copies exact baseline/head sources here.
// Both routines are compiled in the same binary with the same optimization flags.
#[path = "baseline.rs"]
mod baseline;
#[path = "candidate.rs"]
mod candidate;

use std::{hint::black_box, time::Instant};

fn benchmark(mut call: impl FnMut(), iterations: usize) -> f64 {
    for _ in 0..20 {
        call();
    }
    let began = Instant::now();
    for _ in 0..iterations {
        call();
    }
    began.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64
}

fn one(name: &str, text: &str) {
    let baseline = baseline::Lines::new(text);
    let candidate = candidate::Lines::new(text);
    assert_eq!(baseline.count(), candidate.count(), "count {name}");
    for i in 0..baseline.count() {
        assert_eq!(baseline.start(i), candidate.start(i), "{name}, line {i}");
        assert_eq!(baseline.end(i), candidate.end(i), "{name}, line {i}");
    }
    for i in [0, 1, text.len() / 2, text.len().saturating_sub(1), text.len()] {
        assert_eq!(baseline.at(i), candidate.at(i), "{name}, byte {i}");
    }

    let iterations = if text.len() < 200 { 200_000 } else {
        (48_000_000 / text.len()).clamp(5, 30_000)
    };
    let before = || { black_box(baseline::Lines::new(black_box(text))); };
    let after = || { black_box(candidate::Lines::new(black_box(text))); };
    let mut before_samples = Vec::new();
    let mut after_samples = Vec::new();
    for _ in 0..7 {
        before_samples.push(benchmark(before, iterations));
        after_samples.push(benchmark(after, iterations));
        after_samples.push(benchmark(after, iterations));
        before_samples.push(benchmark(before, iterations));
    }
    before_samples.sort_by(f64::total_cmp);
    after_samples.sort_by(f64::total_cmp);
    let original = before_samples[before_samples.len() / 2];
    let candidate = after_samples[after_samples.len() / 2];
    println!(
        "BENCH-LINES name={name} bytes={} count={} baseline_us={original:.6} candidate_us={candidate:.6} delta_pct={:.1}",
        text.len(), baseline.count(), (candidate / original - 1.0) * 100.0
    );
}

fn main() {
    one("empty", "");
    one("single-short", "Глава 140");
    one("short-five-lines", "a\n😀\n中文\nfour\nfive");
    one("single-long-line", &"Русский 漢字 text and emoji 😀. ".repeat(30_000));
    one("markdown-regular", &"# Chapter\nA long line of Russian prose with formatting and symbols.\n".repeat(10_000));
    one("tiny-lines", &"a\n".repeat(100_000));
    one("sparse-lines", &("x".repeat(12_000) + "\n").repeat(50));
    one("multilingual-lines", &"Глава 😀 中文\nNew line 漢字\n".repeat(17_000));
    one("crlf-lines", &"windows\r\ntext\r\n".repeat(20_000));
}
