// Compile this harness against the exact checked-out baseline and PR source:
//   AKULA_SPARSE_IMPL=/absolute/path/sparse.rs rustc -O --edition=2021 ...
// Include the real Rust implementation; no parallel reimplementation of splice.
mod sparse {
    include!(env!("AKULA_SPARSE_IMPL"));
}

use sparse::Sparse;
use std::hint::black_box;
use std::time::Instant;

const CACHED: usize = 2_048;

fn run(kind: &str) -> (f64, u64) {
    let mut cache = Sparse::<usize>::default();
    for line in 0..CACHED {
        cache.insert(line, line + 1);
    }

    // Initial insertion and final verification are outside the timed region.
    let started = Instant::now();
    match kind {
        "one_line" => {
            for iteration in 0..2_048 {
                cache.splice(1_024..=1_024, 1);
                cache.insert(1_024, iteration + 1);
                black_box(cache.get(1_023));
            }
        }
        "sixteen_lines" => {
            for iteration in 0..512 {
                cache.splice(1_020..=1_035, 16);
                for line in 1_020..=1_035 {
                    cache.insert(line, iteration + line);
                }
                black_box(cache.get(1_036));
            }
        }
        "line_delta_control" => {
            for iteration in 0..512 {
                cache.splice(700..=701, 1);
                cache.splice(700..=700, 2);
                cache.insert(700, iteration);
                cache.insert(701, iteration + 1);
                black_box(cache.get(702));
            }
        }
        _ => unreachable!(),
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let mut checksum = 0_u64;
    for line in 0..CACHED {
        checksum = checksum.wrapping_mul(1_099_511_628_211).wrapping_add(
            cache.get(line).copied().unwrap_or(0) as u64
        );
    }
    black_box(checksum);
    (elapsed_ms, checksum)
}

fn main() {
    for kind in ["one_line", "sixteen_lines", "line_delta_control"] {
        let (elapsed_ms, checksum) = run(kind);
        println!("SPARSE_AB case={kind} elapsed_ms={elapsed_ms:.6} checksum={checksum}");
    }
}
