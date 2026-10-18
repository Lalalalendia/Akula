//! Opt-in *release-mode* measurements on actual Akula editor frames.
//!
//! Stage sampling happens on the same runner, same prepared VisualLayerPlan,
//! with serial and two-worker SSE4.2 renderers in ABBA order. This is a test
//! binary, not a measurement of the deployed UI or a production SIMD switch.

use super::*;
use masonry::app::VisualLayerPlan;
use std::time::{Duration, Instant};

const FRAMES: u32 = 32;

#[derive(Clone, Copy)]
struct Sample {
    total_us: f64,
    encode_us: f64,
    raster_us: f64,
    raster_pct: f64,
}

fn draw(
    renderer: &mut Renderer,
    layers: &VisualLayerPlan,
    width: u16,
    height: u16,
    scale: f64,
    full: bool,
    trace_zone: &'static str,
) -> Sample {
    let start = Instant::now();
    {
        let _zone = aq_trace::zone(trace_zone);
        renderer
            .draw_frame(layers, width, height, scale, theme::background(), full)
            .unwrap();
    }
    let total_us = start.elapsed().as_secs_f64() * 1e6;
    let stages = renderer.last_stages;
    let raster_pct =
        100.0 * stages.raster_pixels as f64 / stages.screen_pixels as f64;
    Sample {
        total_us,
        encode_us: stages.encode.as_secs_f64() * 1e6,
        raster_us: stages.raster.as_secs_f64() * 1e6,
        raster_pct,
    }
}

fn quantiles(samples: &[Sample], f: impl Fn(&Sample) -> f64) -> [f64; 2] {
    let values: Vec<f64> = samples.iter().map(f).collect();
    percentiles(&values, [0.50, 0.95])
}

fn describe_difference(case: &str, step: u32, width: u16, lhs: &[u32], rhs: &[u32]) -> (usize, u8) {
    assert_eq!(lhs.len(), rhs.len(), "different framebuffer sizes");
    let mut changed = 0usize;
    let mut one_channel_step = 0usize;
    let mut at_most_two = 0usize;
    let mut max_channel = 0u8;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (usize::MAX, usize::MAX, 0, 0);
    let mut examples = Vec::new();
    for (index, (&a, &b)) in lhs.iter().zip(rhs).enumerate() {
        if a == b {
            continue;
        }
        changed += 1;
        let x = index % usize::from(width);
        let y = index / usize::from(width);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
        let deltas = [0, 8, 16, 24].map(|shift| {
            ((a >> shift) as u8).abs_diff((b >> shift) as u8)
        });
        let peak = deltas.into_iter().max().unwrap_or_default();
        max_channel = max_channel.max(peak);
        if peak <= 1 {
            one_channel_step += 1;
        }
        if peak <= 2 {
            at_most_two += 1;
        }
        if examples.len() < 12 {
            examples.push(format!("({x},{y}):{a:08x}/{b:08x} peak={peak}"));
        }
    }
    if changed != 0 {
        eprintln!(
            "VELLO_DIFF case={case} step={step} changed={changed} \
             one_step={one_channel_step} within_two={at_most_two} \
             max_channel={max_channel} bbox={min_x},{min_y},{max_x},{max_y} \
             examples={examples:?}"
        );
    }
    (changed, max_channel)
}

/// Validate the scroll-copy assumption against two *independent full frames*:
/// pixels inside the moved region which are not re-rasterized in the new
/// frame must already equal the previous full frame translated by dy.
/// This distinguishes stale scene/damage accounting from an incorrect
/// byte-copy primitive or painter clipping.
fn check_full_scene_translation(
    case: &str,
    step: u32,
    width: u16,
    height: u16,
    previous: &[u32],
    current: &[u32],
    moved: &[(u16, u16, u16, u16, i32)],
    rastered: &[(u16, u16, u16, u16)],
) {
    let stride = usize::from(width);
    let mut checked = 0usize;
    let mut mismatched = 0usize;
    let mut worst = 0u8;
    let mut sample = Vec::new();
    for &(x, y, w, h, dy) in moved {
        let shift = dy.unsigned_abs() as usize;
        let (top, bottom) = (usize::from(y), usize::from(y) + usize::from(h));
        if shift >= usize::from(h) {
            continue;
        }
        let dst_start = if dy > 0 { top + shift } else { top };
        let dst_end = if dy < 0 { bottom - shift } else { bottom };
        for py in dst_start..dst_end.min(usize::from(height)) {
            let source_y = if dy > 0 { py - shift } else { py + shift };
            if source_y >= usize::from(height) { continue; }
            for px in usize::from(x)..(usize::from(x) + usize::from(w)).min(stride) {
                let redrawn = rastered.iter().any(|&(rx, ry, rw, rh)| {
                    px >= usize::from(rx)
                        && px < usize::from(rx) + usize::from(rw)
                        && py >= usize::from(ry)
                        && py < usize::from(ry) + usize::from(rh)
                });
                if redrawn { continue; }
                checked += 1;
                let source = previous[source_y * stride + px];
                let target = current[py * stride + px];
                let peak = [0, 8, 16, 24]
                    .into_iter()
                    .map(|bits| ((source >> bits) as u8).abs_diff((target >> bits) as u8))
                    .max()
                    .unwrap_or(0);
                worst = worst.max(peak);
                if peak > 1 {
                    mismatched += 1;
                    if sample.len() < 8 {
                        sample.push(format!("({px},{py}) source_y={source_y} peak={peak}"));
                    }
                }
            }
        }
    }
    if !moved.is_empty() {
        eprintln!(
            "VELLO_SCENE_SHIFT case={case} step={step} checked={checked} \
             mismatched_gt1={mismatched} max_channel={worst} examples={sample:?}"
        );
    }
}

fn trace_marks(case: &str) -> (&'static str, &'static str) {
    match case {
        "text_1x" => ("vello_text_1x_begin", "vello_text_1x_end"),
        "text_175" => ("vello_text_175_begin", "vello_text_175_end"),
        "cover_1x" => ("vello_cover_1x_begin", "vello_cover_1x_end"),
        "cover_175" => ("vello_cover_175_begin", "vello_cover_175_end"),
        _ => panic!("unknown trace case"),
    }
}

fn profile_case(case: &str, scale: f64, width: u32, height: u32, with_cover: bool) {
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(masonry::core::WindowEvent::Resize(
        PhysicalSize::new(width, height),
    ));
    h.app.window_resized(
        &mut h.root,
        Size::new(f64::from(width) / scale, f64::from(height) / scale),
    );
    let body: String = (0..250)
        .map(|i| format!(
            "Абзац {i}: реальный Unicode текст Akula с кириллицей, \
             **жирным**, *курсивом*, ссылками [[Akula]] и длинным \
             предложением для переноса строк через Parley.\n\n"
        ))
        .collect();
    let frontmatter = if with_cover {
        "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n"
    } else {
        ""
    };
    let path = h.write_note("vello-stage-profile.md", &format!("{frontmatter}{body}"));
    h.app.open(&mut h.root, path);
    if with_cover {
        h.wait_cover(true);
    }
    let _ = h.root.redraw();

    let (w, hh) = (width as u16, height as u16);
    let mut serial = Renderer::new(0);
    let mut parallel = Renderer::new(2);
    let mut samples_serial = Vec::new();
    let mut samples_parallel = Vec::new();
    let mut shift_count = 0usize;
    let mut parity_checks = 0usize;
    let mut partial_full_changed = 0usize;
    let mut partial_full_max_channel = 0u8;
    let (start, end) = trace_marks(case);
    let diagnostic = std::env::var_os("AQ_PIXEL_DIAG").is_some();
    let mut first_missed_step = None;
    let mut previous_moved = Vec::new();
    let mut previous_rastered = Vec::new();
    let mut previous_full: Option<Vec<u32>> = None;
    aq_trace::mark(start);
    for step in 0..FRAMES {
        if step % 8 == 7 {
            h.pointer_move(f64::from(70 + step * 8), f64::from(95 + step * 9));
        } else {
            let dy = if step < 20 { -29.0 } else { 23.0 };
            h.scroll(
                (f64::from(width) * 0.6, f64::from(height) * 0.5),
                scale,
                (0.0, dy),
            );
        }
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();
        let serial_first = matches!(step % 4, 0 | 3);
        let (a, b) = if serial_first {
            (
                draw(&mut serial, &layers, w, hh, scale, step == 0, "bench: serial frame"),
                draw(&mut parallel, &layers, w, hh, scale, step == 0, "bench: parallel frame"),
            )
        } else {
            let b = draw(&mut parallel, &layers, w, hh, scale, step == 0, "bench: parallel frame");
            let a = draw(&mut serial, &layers, w, hh, scale, step == 0, "bench: serial frame");
            (a, b)
        };
        let (mismatch, _) = describe_difference(
            case, step, w, serial.frame_pixels(), parallel.frame_pixels(),
        );
        assert_eq!(mismatch, 0, "{case}: serial/parallel pixels disagree on frame {step}");
        // Optional test-only visual artifact capture for sampler comparisons.
        // This is a separate diagnostic invocation, never part of the timing
        // A/B runs; ordinary benchmarking keeps all image I/O disabled.
        if matches!(step, 0 | 20)
            && let Some(path) = std::env::var_os("AQ_SAMPLER_SCREENSHOTS")
        {
            let path = std::path::PathBuf::from(path);
            std::fs::create_dir_all(&path).expect("create sampler image directory");
            std::fs::write(path.join(format!("{case}-step{step}.png")), serial.png())
                .expect("write sampler screenshot");
        }

        // In diagnostic-only mode compare every frame, rather than only
        // the benchmark's four snapshots, to isolate the *first* bad shift.
        // This deliberately does NOT produce performance samples.
        if diagnostic {
            let full = render_full(&layers, w, hh, scale);
            if let Some(previous) = &previous_full {
                check_full_scene_translation(
                    case, step, w, hh, previous, full.frame_pixels(),
                    &serial.moved, &serial.rastered,
                );
            }
            previous_full = Some(full.frame_pixels().to_vec());
            let (bad, peak) = describe_difference(
                case, step, w, serial.frame_pixels(), full.frame_pixels(),
            );
            if peak > 1 {
                first_missed_step.get_or_insert(step);
                eprintln!(
                    "VELLO_PIXEL_DIAG case={case} step={step} bad_pixels={bad} peak={peak} \
                     current_moves={:?} previous_moves={previous_moved:?} \
                     current_raster={:?} previous_raster={previous_rastered:?}",
                    serial.moved, serial.rastered
                );
                if let Some(output) = std::env::var_os("AQ_OUT") {
                    let dir = std::path::PathBuf::from(output);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join(format!("{case}-step{step}-partial.png")), serial.png()).unwrap();
                    std::fs::write(dir.join(format!("{case}-step{step}-full.png")), full.png()).unwrap();
                }
            }
            previous_moved = serial.moved.clone();
            previous_rastered = serial.rastered.clone();
            continue;
        }
        // Never fold pixel-validation work into the raster or frame samples.
        if matches!(step, 0 | 10 | 20 | 31) {
            let full = render_full(&layers, w, hh, scale);
            let (bad, max_delta) = describe_difference(
                case, step, w, serial.frame_pixels(), full.frame_pixels(),
            );
            partial_full_changed += bad;
            partial_full_max_channel = partial_full_max_channel.max(max_delta);
            if bad != 0 {
                let stats = serial.last_stages;
                eprintln!(
                    "VELLO_DIFF_WORK case={case} step={step} raster_pixels={}                      shifted_pixels={} final_rects={} encode_us={:.1} raster_us={:.1}",
                    stats.raster_pixels, stats.shifted_pixels, stats.raster_rects,
                    stats.encode.as_secs_f64() * 1e6,
                    stats.raster.as_secs_f64() * 1e6
                );
                if let Some(output) = std::env::var_os("AQ_OUT") {
                    let dir = std::path::PathBuf::from(output);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(
                        dir.join(format!("{case}-step{step}-partial.png")),
                        serial.png(),
                    ).unwrap();
                    std::fs::write(
                        dir.join(format!("{case}-step{step}-full.png")),
                        full.png(),
                    ).unwrap();
                }
            }
            // Akula's existing Native frame-fidelity helper accepts up
            // to two 8-bit channel steps for independent full redraws.
            // This stricter <=1 diagnostic gate catches geometry/alpha
            // corruption without misreporting existing glyph edge 1-LSB
            // rounding as byte-identical incremental output.
            // Serial/parallel renderer parity above stays byte-exact.
            assert!(
                max_delta <= 1,
                "{case}: partial/full exceeded 1-LSB channel difference on frame {step}: {bad} pixels, max={max_delta}"
            );
            parity_checks += 1;
        }
        if step != 0 {
            samples_serial.push(a);
            samples_parallel.push(b);
            shift_count += serial.last_stages.damage.shifts;
        }
    }
    aq_trace::mark(end);
    if diagnostic {
        eprintln!("VELLO_PIXEL_DIAG_SUMMARY case={case} frames={FRAMES} first_missed_step={first_missed_step:?}");
        return;
    }

    assert_eq!(samples_serial.len(), FRAMES as usize - 1);
    assert_eq!(samples_parallel.len(), FRAMES as usize - 1);
    assert_eq!(parity_checks, 4);
    assert!(shift_count >= 16, "{case}: scroll did not exercise row shifts");

    for (mode, samples) in [
        ("serial", &samples_serial),
        ("parallel", &samples_parallel),
    ] {
        let [draw_p50, draw_p95] = quantiles(samples, |s| s.total_us);
        let [encode_p50, encode_p95] = quantiles(samples, |s| s.encode_us);
        let [raster_p50, raster_p95] = quantiles(samples, |s| s.raster_us);
        let [area_p50, area_p95] = quantiles(samples, |s| s.raster_pct);
        let active = samples.iter().filter(|s| s.raster_us > 0.0).count();
        assert!(active >= 18, "{case}/{mode}: too few raster frames ({active})");
        eprintln!(
            "VELLO_RELEASE case={case} mode={mode} frames={} parity={parity_checks} \
             shifts={shift_count} raster_frames={active} \
             frame_p50_us={draw_p50:.2} frame_p95_us={draw_p95:.2} \
             encode_p50_us={encode_p50:.2} encode_p95_us={encode_p95:.2} \
             raster_p50_us={raster_p50:.2} raster_p95_us={raster_p95:.2} \
             area_p50_pct={area_p50:.3} area_p95_pct={area_p95:.3}",
            samples.len()
        );
    }
    eprintln!(
        "VELLO_RELEASE_PARITY case={case} frames={FRAMES} serial_parallel_diff=0 \
         partial_full_diff_pixels={partial_full_changed} \
         partial_full_max_channel={partial_full_max_channel} \
         partial_full_tolerance=1"
    );
}

#[test]
#[ignore = "opt-in release-mode real-frame trace; no production behavior changes"]
fn benchmark_vello_release_trace() {
    if std::env::var_os("AQ_TRACE").is_some() {
        aq_trace::start();
    }
    if std::env::var_os("AQ_PIXEL_DIAG").is_some() {
        profile_case("text_175", 1.75, 1750, 1051, false);
        return;
    }
    for (case, scale, width, height, cover) in [
        ("text_1x", 1.0, 1000, 600, false),
        ("text_175", 1.75, 1750, 1051, false),
        ("cover_1x", 1.0, 1000, 600, true),
        ("cover_175", 1.75, 1750, 1051, true),
    ] {
        profile_case(case, scale, width, height, cover);
    }
    if let Some(trace) = std::env::var_os("AQ_TRACE") {
        aq_trace::write(std::path::Path::new(&trace)).expect("write Chrome trace");
        eprintln!("VELLO_RELEASE_TRACE written");
    }
}
