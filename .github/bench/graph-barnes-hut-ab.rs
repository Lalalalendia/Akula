// Standalone, no-dependency kernel measurement against the real pinned Rust sources.
// The GitHub workflow passes base/head paths at compile time, not a substitute
// approximation of the Barnes-Hut algorithm.
mod force {
    include!(env!("AKULA_FORCE_IMPL"));
}
mod separate {
    include!(env!("AKULA_SEPARATE_IMPL"));
}
use std::hint::black_box;
use std::time::Instant;

fn fixture(count: usize) -> (Vec<f64>, Vec<(u32, u32)>) {
    let mut points = Vec::with_capacity(count * 2);
    let mut edges = Vec::with_capacity(count * 2);
    for i in 0..count {
        // Deterministic non-coincident positions with spatial clustering:
        // modestly crowded but bounded for the grid-collision control.
        let x = (i.wrapping_mul(9_973) % 1_003) as f64 * 0.041
            + (i % 17) as f64 * 0.09;
        let y = (i.wrapping_mul(7_919) % 991) as f64 * 0.039
            + (i % 23) as f64 * 0.07;
        points.extend_from_slice(&[x, y]);
        edges.push((i as u32, ((i + 1) % count) as u32));
        edges.push((i as u32, ((i + 73) % count) as u32));
    }
    (points, edges)
}

fn checksum(points: &[f64]) -> u64 {
    points.iter().fold(0xCBF2_9CE4_8422_2325_u64, |acc, value| {
        (acc ^ value.to_bits()).wrapping_mul(0x1000_0000_01B3)
    })
}

fn force_case(count: usize, ticks: usize) {
    let (mut points, edges) = fixture(count);
    let settings = force::Settings {
        ticks,
        spacing: 1.0,
        repulsion: 1.0,
    };
    let started = Instant::now();
    force::refine(&mut points, &edges, &settings);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let hash = checksum(black_box(&points));
    assert!(points.iter().all(|n| n.is_finite()));
    println!("GRAPH_BH_AB case=force_{count} elapsed_ms={elapsed_ms:.6} checksum={hash}");
}

fn main() {
    for (count, ticks) in [(1_024, 12), (4_096, 7), (10_000, 4)] {
        force_case(count, ticks);
    }
    let (mut points, _) = fixture(4_096);
    let started = Instant::now();
    separate::separate(&mut points, 0.45, 5);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let hash = checksum(black_box(&points));
    println!("GRAPH_BH_AB case=separate_control elapsed_ms={elapsed_ms:.6} checksum={hash}");
}
