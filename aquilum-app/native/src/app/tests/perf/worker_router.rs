//! Test-only proof of persistent 0/2-worker context routing in real Akula frames.
//!
//! Uses two hot contexts and ONE shared glyph/image Resource cache rather than
//! rebuilding Rayon pools or resources after each scroll/full transition.
//! This cannot change the production behavior unless deliberately promoted.

use super::*;
use masonry::app::VisualLayerPlan;
use std::time::{Duration, Instant};

const FRAMES: u32 = 64;
const ROUTE_THRESHOLD_PX: u64 = 350_000;

fn max_delta(lhs: &[u32], rhs: &[u32]) -> u8 {
    assert_eq!(lhs.len(), rhs.len());
    lhs.iter().zip(rhs).flat_map(|(&a, &b)| {
        [0, 8, 16, 24].map(move |shift| ((a >> shift) as u8).abs_diff((b >> shift) as u8))
    }).max().unwrap_or(0)
}

fn draw_route(
    renderer: &mut Renderer,
    layers: &VisualLayerPlan,
    width: u16,
    height: u16,
    scale: f64,
    full: bool,
) -> f64 {
    let started = Instant::now();
    renderer.draw_frame(layers, width, height, scale, theme::background(), full).unwrap();
    started.elapsed().as_secs_f64() * 1e6
}

fn run_route_case(case: &str, width: u32, height: u32, scale: f64, cover: bool, full_every: u32) {
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)));
    h.app.window_resized(&mut h.root, Size::new(f64::from(width) / scale, f64::from(height) / scale));
    let body: String = (0..250).map(|i| format!(
        "Абзац {i}: текст Akula с кириллицей, **жирным**, *курсивом*, ссылками [[Akula]] \
         и длинным предложением для переноса строк через Parley.\n\n"
    )).collect();
    let fm = if cover {
        "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n"
    } else {
        ""
    };
    let path = h.write_note("dual-dispatch-route.md", &format!("{fm}{body}"));
    h.app.open(&mut h.root, path);
    if cover { h.wait_cover(true); }
    let _ = h.root.redraw();

    let mut renderers = [Renderer::new(0), Renderer::new(2), Renderer::new(2)];
    renderers[2].enable_dual_dispatch_for_test(ROUTE_THRESHOLD_PX);
    let mut totals: [[Vec<f64>; 2]; 3] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
    let (w, hh) = (width as u16, height as u16);
    let mut checks = 0u32;

    for step in 0..FRAMES {
        if step % 8 == 7 {
            h.pointer_move(f64::from(70 + step * 8), f64::from(95 + step * 9));
        } else {
            let dy = if step < 40 { -29.0 } else { 23.0 };
            h.scroll((f64::from(width) * 0.6, f64::from(height) * 0.5), scale, (0.0, dy));
        }
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();
        // Periodic full invalidation forces several actual transitions
        // between small scroll strips and the entire physical frame.
        let full = step == 0 || step % full_every == 0;

        let order = match step % 3 {
            0 => [0, 1, 2],
            1 => [1, 2, 0],
            _ => [2, 0, 1],
        };
        let mut frame_us = [0.0; 3];
        for index in order {
            frame_us[index] = draw_route(&mut renderers[index], &layers, w, hh, scale, full);
        }

        for index in 1..3 {
            assert_eq!(
                renderers[0].frame_pixels(), renderers[index].frame_pixels(),
                "{case}/full_every={full_every}: 0/{index} pixels differ on step {step}"
            );
            assert_eq!(
                renderers[0].last_stages.raster_pixels,
                renderers[index].last_stages.raster_pixels,
                "{case}/step{step}: the route changed scheduled damage"
            );
        }
        if matches!(step, 0 | 10 | 20 | 31 | 47 | 63) {
            let fresh = render_full(&layers, w, hh, scale);
            let peak = max_delta(renderers[2].frame_pixels(), fresh.frame_pixels());
            assert!(
                peak <= 1,
                "{case}/step{step}/full_every={full_every}: routed versus full peak {peak}/255"
            );
            checks += 1;
        }
        if step == 0 {
            continue;
        }
        // Count real scroll candidates, plus explicitly forced full frames.
        // Skip idle pointer-only frames from workload performance medians.
        let scrolling = renderers[0].last_stages.damage.shifts > 0;
        if !full && !scrolling {
            continue;
        }
        let segment = usize::from(full);
        for index in 0..3 {
            totals[index][segment].push(frame_us[index]);
        }
    }

    let switches = renderers[2].dual_dispatch_switches_for_test();
    assert!(switches >= 6, "{case}: expected several context switches, got {switches}");
    assert_eq!(checks, 6);
    for (index, mode) in ["serial", "parallel2", "routed"].into_iter().enumerate() {
        for (segment, class) in ["scroll", "full"].into_iter().enumerate() {
            let samples = &totals[index][segment];
            assert!(samples.len() >= 4, "{case}/{class}/{mode}: too few frames {}", samples.len());
            let [p50, p95] = percentiles(samples, [0.50, 0.95]);
            let total: f64 = samples.iter().sum();
            eprintln!(
                "VELLO_DUAL_ROUTE case={case} full_every={full_every} mode={mode} \
                 workload={class} frames={} route_switches={switches} \
                 p50_us={p50:.2} p95_us={p95:.2} total_us={total:.2}",
                samples.len(),
            );
        }
    }
}

#[test]
#[ignore = "opt-in persistent dual-context route; production worker policy unchanged"]
fn benchmark_vello_dual_dispatch_route() {
    for (case, width, height, scale, cover) in [
        ("text_1x", 1000, 600, 1.0, false),
        ("text_175", 1750, 1051, 1.75, false),
        ("cover_175", 1750, 1051, 1.75, true),
    ] {
        for full_every in [8, 3] {
            run_route_case(case, width, height, scale, cover, full_every);
        }
    }
}


/// Single renderer per PROCESS: unlike the 3-way parity test, this can
/// compare process peak RSS attributable to the additional parked context.
/// Linux /proc VmHWM includes UI/font/allocator memory in all variants; it
/// is not a measure of the extra RenderContext allocation alone.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "Linux release process-level RSS and mixed-frame wall time A/B"]
fn benchmark_vello_single_region_process_rss() {
    const FRAME_COUNT: u32 = 72;
    let mode = std::env::var("AQ_SINGLE_REGION_RSS_MODE").expect("select serial, parallel2 or routed");
    let (threads, routed) = match mode.as_str() {
        "serial" => (0, false),
        "parallel2" => (2, false),
        "routed" => (2, true),
        other => panic!("unknown single-region process mode: {other}"),
    };
    assert_eq!(
        std::env::var("AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT").as_deref() == Ok("1"),
        routed,
        "only the routed variant may enable the production opt-in"
    );
    let (scale, width, height) = (1.75, 1750_u32, 1051_u32);
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)));
    h.app.window_resized(&mut h.root, Size::new(f64::from(width) / scale, f64::from(height) / scale));
    let body: String = (0..250).map(|i| format!(
        "Абзац {i}: текст Akula с кириллицей, **жирным**, *курсивом*, ссылками [[Akula]] \
         и длинным предложением для переноса строк через Parley.\n\n"
    )).collect();
    let path = h.write_note("single-region-process-rss.md", &body);
    h.app.open(&mut h.root, path);
    let _ = h.root.redraw();
    let (w, hh) = (width as u16, height as u16);
    let mut renderer = Renderer::new(threads);
    let mut sampled = Vec::new();
    let (mut actual_shifts, mut full_frames, mut small_region_frames) = (0usize, 0usize, 0usize);
    for step in 0..FRAME_COUNT {
        if step % 8 == 7 {
            h.pointer_move(f64::from(70 + step * 8), f64::from(95 + step * 9));
        } else {
            h.scroll(
                (f64::from(width) * 0.6, f64::from(height) * 0.5),
                scale,
                (0.0, if step < 45 { -29.0 } else { 23.0 }),
            );
        }
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();
        let full = step == 0 || step % 8 == 0;
        let started = Instant::now();
        renderer.draw_frame(&layers, w, hh, scale, theme::background(), full).unwrap();
        let elapsed = started.elapsed().as_secs_f64() * 1e6;
        if step == 0 { continue; }
        actual_shifts += usize::from(renderer.last_stages.damage.shifts > 0);
        full_frames += usize::from(full);
        small_region_frames += usize::from(!full
            && renderer.last_stages.raster_rects == 1
            && renderer.last_stages.raster_pixels <= 350_000
            && renderer.last_stages.raster_pixels > 0);
        if full || renderer.last_stages.damage.shifts > 0 {
            sampled.push(elapsed);
        }
    }
    assert!(actual_shifts >= 40, "not enough natural scroll candidates");
    assert!(full_frames >= 6, "full redrawing not exercised");
    assert!(small_region_frames >= 30, "single small region not exercised");
    let switches = renderer.dual_dispatch_switches_for_test();
    if routed {
        assert!(switches >= 8, "no actual serial/parallel transitions: {switches}");
    } else {
        assert_eq!(switches, 0, "static renderer switched contexts");
    }
    let peak_rss_kib = std::fs::read_to_string("/proc/self/status")
        .expect("proc status for this test process")
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|num| num.parse::<u64>().ok())
        .expect("Linux VmHWM must report process RSS in KiB");
    let [p50, p95] = percentiles(&sampled, [0.50, 0.95]);
    let total: f64 = sampled.iter().sum();
    eprintln!(
        "VELLO_SINGLE_REGION_PROCESS_RSS mode={mode} frames={} shifts={actual_shifts} \
         full_frames={full_frames} small_frames={small_region_frames} switches={switches} \
         peak_kib={peak_rss_kib} p50_us={p50:.2} p95_us={p95:.2} total_us={total:.2}",
        sampled.len(),
    );
}
