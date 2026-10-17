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
    let (start, end) = trace_marks(case);
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
        let mismatch = serial.frame_pixels().iter()
            .zip(parallel.frame_pixels())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(mismatch, 0, "{case}: serial/parallel pixels disagree on frame {step}");

        // Never fold pixel-validation work into the raster or frame samples.
        if matches!(step, 0 | 10 | 20 | 31) {
            let full = render_full(&layers, w, hh, scale);
            let bad = full.frame_pixels().iter()
                .zip(serial.frame_pixels())
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(bad, 0, "{case}: partial/full raster mismatch on frame {step}");
            parity_checks += 1;
        }
        if step != 0 {
            samples_serial.push(a);
            samples_parallel.push(b);
            shift_count += serial.last_stages.damage.shifts;
        }
    }
    aq_trace::mark(end);

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
    eprintln!("VELLO_RELEASE_PARITY case={case} frames={FRAMES} serial_parallel_diff=0 partial_full_diff=0");
}

#[test]
#[ignore = "opt-in release-mode real-frame trace; no production behavior changes"]
fn benchmark_vello_release_trace() {
    if std::env::var_os("AQ_TRACE").is_some() {
        aq_trace::start();
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
