//! Native real-scroll Vello stage-cost study, without synthetic rectangles.
//!
//! Three persistent 0/2/4 worker renderers consume exactly the same Masonry
//! VisualLayerPlan. No test-only damage splitting/guard/forced-full modes.
//! Per-rectangle begin, scene replay, flush and raster stages are timed in
//! isolation. Parity and profiling statistics are outside the draw timer.

use super::*;
use masonry::app::VisualLayerPlan;
use std::time::{Duration, Instant};

const STEPS: u32 = 48;
const WORKERS: [u16; 3] = [0, 2, 4];
type RectU16 = (u16, u16, u16, u16);

#[derive(Clone, Copy)]
struct Observation {
    total_us: f64,
    stages: crate::render::renderer::FrameStages,
    eligible_adjacent: bool,
}

fn adjacent_candidate(regions: &[RectU16]) -> bool {
    if regions.len() < 2 { return false; }
    // Do not treat overlapping translucent damage as safe to coalesce.
    for (i, &(x, y, w, h)) in regions.iter().enumerate() {
        if w == 0 || h == 0 { return false; }
        for &(xx, yy, ww, hh) in &regions[i + 1..] {
            if u32::from(x) < u32::from(xx) + u32::from(ww)
                && u32::from(xx) < u32::from(x) + u32::from(w)
                && u32::from(y) < u32::from(yy) + u32::from(hh)
                && u32::from(yy) < u32::from(y) + u32::from(h)
            {
                return false;
            }
        }
    }
    for (i, &(x, y, w, h)) in regions.iter().enumerate() {
        for &(xx, yy, ww, hh) in &regions[i + 1..] {
            if (y == yy && h == hh
                && (u32::from(x) + u32::from(w) == u32::from(xx)
                    || u32::from(xx) + u32::from(ww) == u32::from(x)))
                || (x == xx && w == ww
                    && (u32::from(y) + u32::from(h) == u32::from(yy)
                        || u32::from(yy) + u32::from(hh) == u32::from(y)))
            {
                return true;
            }
        }
    }
    false
}

fn sample(
    renderer: &mut Renderer,
    layers: &VisualLayerPlan,
    w: u16,
    h: u16,
    scale: f64,
    full: bool,
) -> Observation {
    let begun = Instant::now();
    renderer
        .draw_frame(layers, w, h, scale, theme::background(), full)
        .expect("natural scroll frame");
    let total_us = begun.elapsed().as_secs_f64() * 1e6;
    Observation {
        total_us,
        stages: renderer.last_stages,
        eligible_adjacent: adjacent_candidate(&renderer.rastered),
    }
}

fn max_channel_delta(a: &[u32], b: &[u32]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b)
        .flat_map(|(&a, &b)| {
            [0, 8, 16, 24].map(move |bits| ((a >> bits) as u8).abs_diff((b >> bits) as u8))
        })
        .max()
        .unwrap_or(0)
}

fn run_case(name: &str, scale: f64, width: u32, height: u32, cover: bool) {
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(
        masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)),
    );
    h.app.window_resized(
        &mut h.root,
        Size::new(f64::from(width) / scale, f64::from(height) / scale),
    );
    // Same test content as the existing Damage Tracker real-frame profile.
    let body: String = (0..200).map(|i| format!(
        "Абзац {i}: Unicode 😀 и русский текст; ссылки [[Akula]] и \
         **разметка** для реального прохода Masonry, \
         переноса строк и частичной перерисовки.\n\n"
    )).collect();
    let frontmatter = if cover {
        "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n"
    } else {
        ""
    };
    let note = h.write_note("natural-vello-profile.md", &format!("{frontmatter}{body}"));
    h.app.open(&mut h.root, note);
    if cover {
        h.wait_cover(true);
    }

    let (w, hh) = (width as u16, height as u16);
    let mut renderers = [
        Renderer::new(WORKERS[0]),
        Renderer::new(WORKERS[1]),
        Renderer::new(WORKERS[2]),
    ];
    let mut values: [Vec<Observation>; 3] =
        std::array::from_fn(|_| Vec::with_capacity(STEPS as usize));
    let mut parity_checks = 0usize;

    for step in 0..STEPS {
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
        // Rotating execution order controls thermal/cache order effects.
        let order = match step % 3 {
            0 => [0, 1, 2],
            1 => [1, 2, 0],
            _ => [2, 0, 1],
        };
        let mut observed = [None; 3];
        for index in order {
            observed[index] = Some(sample(
                &mut renderers[index], &layers, w, hh, scale, step == 0,
            ));
        }
        let frame = observed.map(|x| x.expect("each renderer must draw"));

        for index in 1..3 {
            assert_eq!(
                renderers[0].frame_pixels(),
                renderers[index].frame_pixels(),
                "{name} step={step}: identical native regions across workers must be bitwise equal"
            );
            assert_eq!(
                frame[0].stages.raster_rects,
                frame[index].stages.raster_rects,
                "{name} step={step}: workers disagree on region count"
            );
            assert_eq!(
                frame[0].stages.raster_pixels,
                frame[index].stages.raster_pixels,
                "{name} step={step}: workers disagree on damaged pixel area"
            );
        }
        if step % 12 == 0 {
            let full_ref = render_full(&layers, w, hh, scale);
            let max = max_channel_delta(renderers[0].frame_pixels(), full_ref.frame_pixels());
            assert!(
                max <= 1,
                "{name} step={step}: natural scroll/full reference delta {max}/255"
            );
            parity_checks += 1;
        }
        // Exclude setup and pointer-only steps, retaining only actual scroll
        // candidates from the Damage Tracker. Do not fabricate fragmentation.
        if step > 0 && frame[0].stages.damage.shifts > 0 {
            for i in 0..3 {
                values[i].push(frame[i]);
            }
        }
    }
    assert_eq!(parity_checks, 4);

    for (index, threads) in WORKERS.into_iter().enumerate() {
        let samples = &values[index];
        assert!(samples.len() >= 25, "{name}: too few natural scroll frames");
        let get = |f: fn(&Observation) -> f64| -> [f64; 2] {
            let durations: Vec<f64> = samples.iter().map(f).collect();
            percentiles(&durations, [0.50, 0.95])
        };
        let [total50, total95] = get(|v| v.total_us);
        let [record50, record95] = get(|v| v.stages.record.as_secs_f64() * 1e6);
        let [detect50, detect95] = get(|v| v.stages.detect.as_secs_f64() * 1e6);
        let [reset50, reset95] = get(|v| v.stages.reset.as_secs_f64() * 1e6);
        let [replay50, replay95] = get(|v| v.stages.replay.as_secs_f64() * 1e6);
        let [flush50, flush95] = get(|v| v.stages.flush.as_secs_f64() * 1e6);
        let [raster50, raster95] = get(|v| v.stages.raster.as_secs_f64() * 1e6);
        let [area50, area95] = get(|v| 100.0 * v.stages.raster_pixels as f64 / v.stages.screen_pixels as f64);
        let max_rects = samples.iter().map(|s| s.stages.raster_rects).max().unwrap_or(0);
        let multi_rect = samples.iter().filter(|s| s.stages.raster_rects > 1).count();
        let eligible = samples.iter().filter(|s| s.eligible_adjacent).count();
        let total_sum: f64 = samples.iter().map(|s| s.total_us).sum();
        eprintln!(
            "VELLO_NATURAL_STAGES case={name} workers={threads} frames={} \
             max_rects={max_rects} multi_rect_frames={multi_rect} \
             eligible_adjacent_frames={eligible} \
             total_p50_us={total50:.2} total_p95_us={total95:.2} \
             record_p50_us={record50:.2} record_p95_us={record95:.2} \
             detect_p50_us={detect50:.2} detect_p95_us={detect95:.2} \
             reset_p50_us={reset50:.2} reset_p95_us={reset95:.2} \
             replay_p50_us={replay50:.2} replay_p95_us={replay95:.2} \
             flush_p50_us={flush50:.2} flush_p95_us={flush95:.2} \
             raster_p50_us={raster50:.2} raster_p95_us={raster95:.2} \
             raster_area_p50_pct={area50:.3} raster_area_p95_pct={area95:.3} \
             total_sum_us={total_sum:.2}",
            samples.len(),
        );
    }
}

#[test]
#[ignore = "opt-in native real-scroll Vello stage breakdown, production unchanged"]
fn benchmark_vello_natural_scroll_stages() {
    // Named cases deliberately align with the existing Damage Tracker fixture.
    run_case("cover_1x", 1.0, 1000, 600, true);
    run_case("cover_175", 1.75, 1750, 1051, true);
    run_case("plain_175", 1.75, 1750, 1051, false);
}
