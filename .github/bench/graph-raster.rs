// Standalone, dependency-free graph-raster kernel benchmark.
// CI copies the same source into both baseline and candidate trees.
#[path = "raster/mod.rs"]
mod raster;

use raster::{Disc, bands, dilate};
use std::hint::black_box;
use std::time::Instant;

fn median_ms(repeats: usize, mut run: impl FnMut()) -> f64 {
    for _ in 0..3 {
        run();
    }
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..repeats {
            run();
        }
        samples.push(start.elapsed().as_secs_f64() * 1000.0 / repeats as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[3]
}

fn render(discs: &[Disc], width: usize, height: usize, pixels: &mut [u8]) {
    pixels.fill(31);
    bands(width, height, pixels, |band| {
        for disc in discs {
            band.disc(disc);
        }
    });
}

fn main() {
    let (width, height) = (512usize, 512usize);
    let mut discs: Vec<Disc> = (0..360u32)
        .map(|i| Disc {
            center: [
                ((i * 103 + 17) % 570) as f32 - 29.0,
                ((i * 67 + 11) % 540) as f32 - 14.0,
            ],
            radius: 11.0 + (i % 13) as f32 * 2.5,
            inner: 0.0,
            body: [180, 90, 20],
            rim: [40, 70, 230],
            alpha: 0.73,
        })
        .collect();
    let mut pixels = vec![0u8; width * height * 4];
    for (name, proportion) in [
        ("mostly_solid_discs", 0.94f32),
        ("medium_rim_discs", 0.55f32),
        ("thin_body_discs", 0.15f32),
    ] {
        for disc in &mut discs {
            disc.inner = disc.radius * proportion;
        }
        let result = median_ms(8, || {
            render(&discs, width, height, &mut pixels);
            black_box(&pixels);
        });
        let checksum: u64 = pixels.iter().map(|b| u64::from(*b)).sum();
        println!("BENCH-RASTER name={name} median_ms={result:.6} checksum={checksum}");
    }

    let (mask_width, mask_height) = (1024usize, 768usize);
    let mask: Vec<u8> = (0..mask_width * mask_height)
        .map(|i| ((i * 13 + (i / 11) * 23) % 256) as u8)
        .collect();
    let mut last = Vec::new();
    let result = median_ms(8, || {
        last = dilate(&mask, mask_width, mask_height);
        black_box(&last);
    });
    let checksum: u64 = last.iter().map(|b| u64::from(*b)).sum();
    println!("BENCH-RASTER name=halo_dilate median_ms={result:.6} checksum={checksum}");
}
