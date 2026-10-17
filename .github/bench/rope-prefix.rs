// Identical benchmark fixture compiled against original and candidate rope.rs.
// This targets the actual crop::Rope implementation, not simulated chunks.
#[path = "rope.rs"]
mod rope;

use crop::Rope;
use std::{hint::black_box, time::Instant};

fn boundary_before(s: &str, offset: usize) -> usize {
    let mut offset = offset.min(s.len());
    while !s.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn measure(a: &Rope, b: &Rope, repeats: usize, expected: usize) -> f64 {
    assert_eq!(rope::common_prefix(a, b), expected);
    for _ in 0..8 {
        black_box(rope::common_prefix(black_box(a), black_box(b)));
    }
    let mut samples = Vec::with_capacity(9);
    for _ in 0..9 {
        let now = Instant::now();
        for _ in 0..repeats {
            black_box(rope::common_prefix(black_box(a), black_box(b)));
        }
        samples.push(now.elapsed().as_secs_f64() * 1_000_000.0 / repeats as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[4]
}

fn main() {
    let large = "Глава 140. Русский текст, 漢字 и 😀. Небольшие изменения.\n".repeat(2800);
    let ascii = "The quick brown fox jumped over the lazy dog.\n".repeat(3200);
    let mut data = vec![
        ("identical-unicode", large.clone(), large.clone()),
        ("identical-ascii", ascii.clone(), ascii.clone()),
        ("short-equal", "Маленькая строка 😀".into(), "Маленькая строка 😀".into()),
    ];

    for (name, share) in [
        ("near-start", 0.0),
        ("middle-edit", 0.5),
        ("near-tail", 0.99),
    ] {
        let mut changed = large.clone();
        changed.insert_str(boundary_before(&large, (large.len() as f64 * share) as usize), "Z");
        data.push((name, large.clone(), changed));
    }

    let mut ascii_changed = ascii.clone();
    ascii_changed.insert_str(boundary_before(&ascii, ascii.len() - 29), "Z");
    data.push(("ascii-tail-edit", ascii.clone(), ascii_changed));

    data.push(("prefix-shorter", large.clone(), large[..boundary_before(&large, large.len() / 3)].to_string()));
    for (name, original, changed) in data {
        let expected = original.as_bytes().iter().zip(changed.as_bytes())
            .take_while(|(a, b)| a == b).count();
        let a = Rope::from(original.as_str());
        let b = Rope::from(changed.as_str());
        let bytes = original.len();
        let repeats = if bytes < 500 { 75000 } else { 120 };
        let elapsed = measure(&a, &b, repeats, expected);
        println!(
            "BENCH-ROPE case={name} bytes={bytes} matched={expected} median_us={elapsed:.6}"
        );
    }
}
