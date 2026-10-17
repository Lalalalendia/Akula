//! Opt-in stage profiler for Akula's real scroll frames.
//! This is diagnostic only. It neither modifies damage decisions nor production
//! rendering, and it never treats isolated hot-path time as end-to-end latency.

use super::*;
use std::time::Duration;

fn run_damage_profile(name: &str, scale: f64, width: u32, height: u32) {
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)));
    h.app.window_resized(
        &mut h.root,
        Size::new(f64::from(width) / scale, f64::from(height) / scale),
    );
    let body: String = (0..200)
        .map(|i| {
            format!(
                "Абзац {i}: Unicode 😀 и русский текст; ссылки [[Akula]] и \
                 **разметка** для реального прохода Masonry, \
                 переноса строк и частичной перерисовки.\n\n"
            )
        })
        .collect();
    let note = h.write_note(
        "Damage profiler.md",
        &format!("---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n{body}"),
    );
    h.app.open(&mut h.root, note);
    h.wait_cover(true);

    let (w, hh) = (width as u16, height as u16);
    let mut renderer = Renderer::new(2);
    let mut samples = Vec::new();
    let mut shifted = 0usize;
    let mut parity_checks = 0usize;

    for step in 0..48u32 {
        if step % 7 == 6 {
            h.pointer_move(f64::from(40 + step * 7), f64::from(80 + step * 11));
        } else {
            let dy = if step < 29 { -37.0 } else { 23.0 };
            h.scroll(
                (f64::from(width) * 0.6, f64::from(height) * 0.5),
                scale,
                (0.0, dy),
            );
        }
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();
        renderer.draw_frame(&layers, w, hh, scale, theme::background(), step == 0).unwrap();
        let sample = renderer.last_stages;
        shifted += sample.damage.shifts;
        samples.push(sample);

        // Pixel parity is deliberately outside measured phases.
        if step % 12 == 0 {
            let full = render_full(&layers, w, hh, scale);
            let unequal = differing_pixels(renderer.frame_pixels(), full.frame_pixels());
            assert_eq!(unequal, 0, "{name}: partial/full mismatch on step {step}");
            parity_checks += 1;
        }
    }

    assert_eq!(samples.len(), 48);
    assert_eq!(parity_checks, 4);
    assert!(shifted >= 12, "{name}: profiler did not exercise row shifts ({shifted})");

    macro_rules! pct {
        ($($field:ident).+) => {{
            let values: Vec<f64> = samples
                .iter()
                .map(|s| s.$($field).+.as_secs_f64() * 1_000_000.0)
                .collect();
            percentiles(&values, [0.50, 0.95])
        }};
    }
    let [record_p50, record_p95] = pct!(record);
    let [detect_p50, detect_p95] = pct!(detect);
    let [sort_p50, sort_p95] = pct!(damage.sort);
    let [compare_p50, compare_p95] = pct!(damage.compare);
    let [moves_p50, moves_p95] = pct!(damage.moves);
    let [merge_p50, merge_p95] = pct!(damage.merge);
    let [encode_p50, encode_p95] = pct!(encode);
    let [raster_p50, raster_p95] = pct!(raster);
    let max_draws = samples.iter().map(|s| s.damage.draws).max().unwrap_or(0);
    let max_changed = samples.iter().map(|s| s.damage.changed).max().unwrap_or(0);
    let max_rects = samples.iter().map(|s| s.damage.raw_rects).max().unwrap_or(0);
    let max_areas = samples.iter().map(|s| s.damage.final_areas).max().unwrap_or(0);
    let max_regions = samples.iter().map(|s| s.damage.regions).max().unwrap_or(0);
    let total_shifts: usize = samples.iter().map(|s| s.damage.shifts).sum();

    eprintln!(
        "DAMAGE_PROFILE case={name} frames=48 parity=4 shifts={total_shifts} \
         record_p50_us={record_p50:.2} record_p95_us={record_p95:.2} \
         detect_p50_us={detect_p50:.2} detect_p95_us={detect_p95:.2} \
         sort_p50_us={sort_p50:.2} sort_p95_us={sort_p95:.2} \
         compare_p50_us={compare_p50:.2} compare_p95_us={compare_p95:.2} \
         moves_p50_us={moves_p50:.2} moves_p95_us={moves_p95:.2} \
         merge_p50_us={merge_p50:.2} merge_p95_us={merge_p95:.2} \
         encode_p50_us={encode_p50:.2} encode_p95_us={encode_p95:.2} \
         raster_p50_us={raster_p50:.2} raster_p95_us={raster_p95:.2} \
         max_draws={max_draws} max_changed={max_changed} \
         max_raw_rects={max_rects} max_areas={max_areas} max_regions={max_regions}"
    );
    // The first frame is deliberately full; exclude it from the partial
    // raster coverage distribution so the normal UI workload is not skewed.
    let partial = &samples[1..];
    assert!(partial.iter().all(|s| s.screen_pixels == u64::from(w) * u64::from(hh)));
    let coverage_pct: Vec<f64> = partial.iter().map(|s| {
        100.0 * s.raster_pixels as f64 / s.screen_pixels as f64
    }).collect();
    let shifted_pct: Vec<f64> = partial.iter().map(|s| {
        100.0 * s.shifted_pixels as f64 / s.screen_pixels as f64
    }).collect();
    let [coverage_p50, coverage_p95] = percentiles(&coverage_pct, [0.5, 0.95]);
    let [shifted_p50, shifted_p95] = percentiles(&shifted_pct, [0.5, 0.95]);
    let max_coverage = coverage_pct.iter().copied().fold(0.0f64, f64::max);
    let max_final_rects = partial.iter().map(|s| s.raster_rects).max().unwrap_or(0);
    let empty_rasters = partial.iter().filter(|s| s.raster_pixels == 0).count();
    let total_raster_pixels: u64 = partial.iter().map(|s| s.raster_pixels).sum();
    assert!(total_raster_pixels > 0, "{name}: no rasterized pixels in scroll workload");
    eprintln!(
        "DAMAGE_COVERAGE case={name} frames=47 screen_pixels={} \\
         raster_p50_pct={coverage_p50:.3} raster_p95_pct={coverage_p95:.3} \\
         raster_max_pct={max_coverage:.3} shifted_p50_pct={shifted_p50:.3} \\
         shifted_p95_pct={shifted_p95:.3} max_final_rects={max_final_rects} \\
         empty_rasters={empty_rasters} total_raster_pixels={total_raster_pixels}",
        u64::from(w) * u64::from(hh)
    );
    eprintln!("DAMAGE_PARITY case={name} frames=48 checked=4 pixel_differences=0");
}

#[test]
#[ignore = "opt-in real-frame profiling; test instrumentation has nonzero overhead"]
fn benchmark_damage_frame_stages() {
    run_damage_profile("standard_1x", 1.0, 1000, 600);
    run_damage_profile("fractional_1_75x", 1.75, 1750, 1051);
}
