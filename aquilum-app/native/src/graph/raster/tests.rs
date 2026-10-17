use super::{Band, Disc, Edge, Sprite, Stamp, bands, blend, dilate};

fn at(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 3] {
    let i = (y * width + x) * 4;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

#[test]
fn draws_discs_lines_and_sprites_in_every_band() {
    let sprite = Sprite { width: 2, height: 2, fill: vec![255; 4], halo: vec![0; 4] };
    let edge = Edge { from: [10.5, 10.5], to: [290.5, 190.5], color: [0, 255, 0], alpha: 1.0 };
    let disc = Disc { center: [150.0, 100.0], radius: 20.0, body: [255, 0, 0], rim: [255, 255, 255], inner: 18.0, alpha: 1.0 };
    let stamp = Stamp { sprite: &sprite, x: 5, y: 130, fill: [0, 0, 255], halo: [0, 0, 0], alpha: 1.0 };
    let mut pixels = vec![0u8; 300 * 200 * 4];
    bands(300, 200, &mut pixels, |band| {
        band.line(&edge);
        band.disc(&disc);
        band.stamp(&stamp);
    });
    assert_eq!(at(&pixels, 300, 150, 100), [255, 0, 0]);
    assert_eq!(at(&pixels, 300, 150, 81), [255, 255, 255]);
    assert_eq!(at(&pixels, 300, 5, 130), [0, 0, 255]);
    assert!(at(&pixels, 300, 270, 177)[1] > 100 || at(&pixels, 300, 270, 178)[1] > 100);
    assert_eq!(at(&pixels, 300, 299, 0), [0, 0, 0]);
}


// Frozen scalar baseline: this reference is deliberately independent of the
// optimized implementation. Exact RGB(A) bytes are the regression oracle.
fn reference_disc(band: &mut Band<'_>, d: &Disc) {
    let [cx, cy] = d.center;
    if d.radius < 1.0 {
        let area = (std::f32::consts::PI * d.radius * d.radius).min(1.0) * d.alpha;
        let (x, y) = (cx - 0.5, cy - 0.5);
        let (bx, by) = (x.floor(), y.floor());
        let (fx, fy) = (x - bx, y - by);
        let (bx, by) = (bx as i32, by as i32);
        band.plot(bx, by, d.body, area * (1.0 - fx) * (1.0 - fy));
        band.plot(bx + 1, by, d.body, area * fx * (1.0 - fy));
        band.plot(bx, by + 1, d.body, area * (1.0 - fx) * fy);
        band.plot(bx + 1, by + 1, d.body, area * fx * fy);
        return;
    }
    let r = d.radius;
    let first = ((cy - r - 1.0).floor() as i64).max(band.rows.start as i64);
    let last = ((cy + r + 1.0).ceil() as i64).min(band.rows.end as i64 - 1);
    let outer = r + 0.5;
    for y in first..=last {
        let dy = y as f32 + 0.5 - cy;
        if dy.abs() > outer {
            continue;
        }
        let span = (outer * outer - dy * dy).max(0.0).sqrt();
        let x_from = ((cx - span).floor() as i64).max(0);
        let x_to = ((cx + span).ceil() as i64).min(band.width as i64 - 1);
        let row = (y as usize - band.rows.start) * band.width;
        for x in x_from..=x_to {
            let dx = x as f32 + 0.5 - cx;
            let distance = (dx * dx + dy * dy).sqrt();
            let coverage = (outer - distance).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let at = (row + x as usize) * 4;
            let px = &mut band.pixels[at..at + 4];
            if d.inner < r {
                let body = (d.inner + 0.5 - distance).clamp(0.0, 1.0);
                if body < 1.0 {
                    blend(px, d.rim, coverage * d.alpha);
                }
                blend(px, d.body, body * coverage * d.alpha);
            } else {
                blend(px, d.body, coverage * d.alpha);
            }
        }
    }
}


fn reference_dilate(mask: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; mask.len()];
    for y in 0..height {
        for x in 0..width {
            let mut best = 0u8;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height {
                        best = best.max(mask[ny as usize * width + nx as usize]);
                    }
                }
            }
            out[y * width + x] = best;
        }
    }
    out
}


fn check_disc_matches_baseline(disc: Disc, initial: &[u8], width: usize, height: usize) {
    let mut old = initial.to_vec();
    let mut new = initial.to_vec();
    bands(width, height, &mut old, |band| reference_disc(band, &disc));
    bands(width, height, &mut new, |band| band.disc(&disc));
    if old != new {
        let i = old.iter().zip(&new).position(|(a, b)| a != b).expect("mismatch position");
        panic!("disc {disc:?}: byte {i}, pixel ({}, {}), channel {}, expected {}, got {}",
            (i / 4) % width, (i / 4) / width, i % 4, old[i], new[i]);
    }
}

#[test]
fn optimized_discs_are_byte_identical_to_scalar_baseline() {
    let (width, height) = (117usize, 139usize); // crosses 64-row raster bands
    let mut initial = vec![0u8; width * height * 4];
    for (i, pixel) in initial.chunks_exact_mut(4).enumerate() {
        pixel[0] = (i as u8).wrapping_mul(19);
        pixel[1] = (i as u8).wrapping_mul(43);
        pixel[2] = (i as u8).wrapping_mul(73);
        pixel[3] = 255;
    }
    for radius in [0.25f32, 0.99, 1.0, 1.01, 1.5, 2.25, 4.0, 16.0, 32.5, 49.0, 64.0] {
        for center in [[-1.75, -1.0], [0.5, 63.5], [55.25, 64.25],
            [116.7, 138.5], [68.0, 123.0]] {
            for inner in [0.0, 0.3 * radius, (radius - 1.25).max(0.0),
                radius * 0.95, radius, radius + 4.0] {
                for alpha in [0.0, 0.07, 0.5, 1.0] {
                    check_disc_matches_baseline(
                        Disc { center, radius, inner, alpha,
                            body: [255, 71, 19], rim: [13, 212, 93] },
                        &initial, width, height,
                    );
                }
            }
        }
    }
    // Deterministic random fractional positions and low/high-opacity blends.
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) as u32
    };
    for _ in 0..350 {
        let radius = 1.0 + (next() % 7200) as f32 / 100.0;
        let center = [
            (next() % 13000) as f32 / 100.0 - 6.0,
            (next() % 15100) as f32 / 100.0 - 6.0,
        ];
        let inner = radius * (next() % 121) as f32 / 100.0;
        let alpha = (next() % 1001) as f32 / 1000.0;
        check_disc_matches_baseline(
            Disc { center, radius, inner, alpha,
                body: [91, 201, 7], rim: [218, 13, 222] },
            &initial, width, height,
        );
    }
}

#[test]
fn separable_halo_dilation_is_byte_identical_to_scalar_baseline() {
    for width in [0usize, 1, 2, 3, 7, 17, 64, 129] {
        for height in [0usize, 1, 2, 3, 9, 61, 65] {
            for variant in 0usize..7 {
                // Keep the old zero-dimension and oversized-buffer contracts.
                let count = width * height + (variant % 3);
                let mask: Vec<u8> = (0..count)
                    .map(|i| ((i * 31 + (i / 7) * 97 + variant * 11) % 256) as u8)
                    .collect();
                assert_eq!(
                    dilate(&mask, width, height),
                    reference_dilate(&mask, width, height),
                    "width={width}, height={height}, variant={variant}"
                );
            }
        }
    }
}

#[test]
fn assembled_graph_frame_matches_frozen_raster() {
    // A complete graph-like composition catches blending-order, clipping and
    // band-crossing differences not exercised by individual disc comparisons.
    let (width, height) = (321usize, 193usize);
    let nodes: Vec<Disc> = (0..96u32)
        .map(|i| {
            let radius = 1.0 + (i % 13) as f32 * 2.9;
            // Match the real GraphView rim range: 0..=0.35 of the radius.
            let rim_share = if radius < 2.5 { 0.0 } else { (1.8 / radius).min(0.35) };
            Disc {
                center: [
                    ((i * 91 + 11) % 360) as f32 - 15.75,
                    ((i * 57 + 31) % 224) as f32 - 12.25,
                ],
                radius,
                inner: radius * (1.0 - rim_share),
                body: [29, 185, 220],
                rim: [210, 45, 80],
                alpha: if i % 4 == 0 { 0.35 } else { 1.0 },
            }
        })
        .collect();
    let edges: Vec<Edge> = (0..150u32)
        .map(|i| Edge {
            from: [
                ((i * 43 + 9) % 360) as f32 - 14.0,
                ((i * 17 + 5) % 220) as f32 - 10.0,
            ],
            to: [
                ((i * 71 + 55) % 360) as f32 - 12.0,
                ((i * 53 + 40) % 220) as f32 - 11.0,
            ],
            color: [90, 105, 110],
            alpha: 0.16,
        })
        .collect();
    let fill: Vec<u8> = (0..27usize * 9)
        .map(|i| if i % 3 == 0 || i % 11 == 0 { 255 } else { 0 })
        .collect();
    let halo = reference_dilate(&fill, 27, 9);
    assert_eq!(dilate(&fill, 27, 9), halo);
    let sprite = Sprite { width: 27, height: 9, fill, halo };
    let stamps = [
        Stamp { sprite: &sprite, x: -4, y: 59, fill: [240, 240, 240], halo: [15, 15, 15], alpha: 0.8 },
        Stamp { sprite: &sprite, x: 125, y: 125, fill: [240, 240, 240], halo: [15, 15, 15], alpha: 0.45 },
        Stamp { sprite: &sprite, x: 308, y: 185, fill: [240, 240, 240], halo: [15, 15, 15], alpha: 1.0 },
    ];

    let mut original = vec![0x29; width * height * 4];
    let mut optimized = original.clone();
    bands(width, height, &mut original, |band| {
        for edge in &edges {
            band.line(edge);
        }
        for disc in &nodes {
            reference_disc(band, disc);
        }
        for stamp in &stamps {
            band.stamp(stamp);
        }
    });
    bands(width, height, &mut optimized, |band| {
        for edge in &edges {
            band.line(edge);
        }
        for disc in &nodes {
            band.disc(disc);
        }
        for stamp in &stamps {
            band.stamp(stamp);
        }
    });
    assert_eq!(original, optimized, "graph scene RGBA pixels differ");
}
