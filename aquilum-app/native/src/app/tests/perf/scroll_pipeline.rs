//! Opt-in Linux input-to-CPU-frame scroll A/B. No production code changes.
//!
//! One real Renderer::new(2) per process. AKULA_VELLO_SINGLE_RECT=0 uses the
//! production rollback path; =1 uses the shipped adaptive constructor.
//! Times a real Masonry pointer event + animation event, redraw/layout/scene,
//! and Akula draw_frame separately. No window surface, compositor, vsync,
//! presentation, or user-perceived FPS is claimed.

use super::*;
use std::time::{Duration, Instant};

const FRAMES: u32 = 72;
const PARITY_STEPS: [u32; 6] = [0, 10, 20, 35, 56, 71];

#[derive(Default)]
struct PipelineTimes {
    input: Vec<f64>,
    scene: Vec<f64>,
    draw: Vec<f64>,
    total: Vec<f64>,
}

fn us(time: Duration) -> f64 {
    time.as_secs_f64() * 1_000_000.0
}

fn peak_channel_delta(a: &[u32], b: &[u32]) -> u8 {
    assert_eq!(a.len(), b.len(), "different frame dimensions");
    a.iter()
        .zip(b)
        .flat_map(|(&left, &right)| {
            [0, 8, 16, 24].map(move |shift| {
                ((left >> shift) as u8).abs_diff((right >> shift) as u8)
            })
        })
        .max()
        .unwrap_or(0)
}

fn frame_digest(mut digest: u64, frame: &[u32]) -> u64 {
    // Stable across independent processes; computed OUTSIDE timed frames.
    for &pixel in frame {
        digest ^= u64::from(pixel);
        digest = digest.wrapping_mul(0x100000001b3);
    }
    digest
}

fn case(mode: &str, label: &str, width: u32, height: u32, scale: f64, cover: bool) {
    let mut h = Harness::with(None, scale);
    h.root
        .handle_window_event(masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)));
    h.app.window_resized(
        &mut h.root,
        Size::new(f64::from(width) / scale, f64::from(height) / scale),
    );

    let body: String = (0..250)
        .map(|i| format!(
            "Абзац {i}: текст Akula с кириллицей, **жирным**, *курсивом*, \
             ссылками [[Akula]] и длинным предложением для переноса строк \
             через Parley.\n\n"
        ))
        .collect();
    let frontmatter = if cover {
        "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n"
    } else {
        ""
    };
    let note = h.write_note("scroll-pipeline.md", &format!("{frontmatter}{body}"));
    h.app.open(&mut h.root, note);
    if cover {
        h.wait_cover(true);
    }

    // Warm both the UI scene and the renderer outside the measurement.
    let (w, hh) = (width as u16, height as u16);
    let (layers, _) = h.root.redraw();
    let mut renderer = Renderer::new(2);
    renderer
        .draw_frame(&layers, w, hh, scale, theme::background(), true)
        .expect("warm scroll renderer");
    let starting_switches = renderer.dual_dispatch_switches_for_test();

    let mut samples = PipelineTimes::default();
    let (mut shift_frames, mut full_frames, mut small_frames, mut checked) =
        (0_u32, 0_u32, 0_u32, 0_u32);
    let mut digest: u64 = 0xcbf29ce484222325;

    for step in 0..FRAMES {
        let start = Instant::now();
        if step % 8 == 7 {
            h.pointer_move(f64::from(70 + step * 8), f64::from(95 + step * 5));
        } else {
            let dy = if step < 45 { -29.0 } else { 23.0 };
            h.scroll(
                (f64::from(width) * 0.6, f64::from(height) * 0.5),
                scale,
                (0.0, dy),
            );
        }
        h.anim(Duration::from_millis(16));
        let input = us(start.elapsed());

        let scene_started = Instant::now();
        let (layers, _) = h.root.redraw();
        let scene = us(scene_started.elapsed());

        let force_full = step == 0 || step % 8 == 0;
        let draw_started = Instant::now();
        renderer
            .draw_frame(&layers, w, hh, scale, theme::background(), force_full)
            .expect("scroll pipeline frame");
        let draw = us(draw_started.elapsed());
        let total = us(start.elapsed());
        let stages = renderer.last_stages;

        // Independent full frame references and hashes never enter timings.
        if PARITY_STEPS.contains(&step) {
            let reference = render_full(&layers, w, hh, scale);
            let delta = peak_channel_delta(renderer.frame_pixels(), reference.frame_pixels());
            assert!(
                delta <= 1,
                "{label}/{mode}/step={step}: partial/full max delta {delta}/255"
            );
            digest = frame_digest(digest, renderer.frame_pixels());
            checked += 1;
        }

        if step == 0 {
            continue; // Setup frame is not steady-state work.
        }
        shift_frames += u32::from(stages.damage.shifts > 0);
        full_frames += u32::from(force_full);
        small_frames += u32::from(
            !force_full
                && stages.raster_rects == 1
                && (1..=350_000).contains(&stages.raster_pixels),
        );

        // Mirror the existing real-scroll sample selection: retain scroll
        // candidates plus forced full frames, but exclude pointer-only idle.
        if force_full || stages.damage.shifts > 0 {
            samples.input.push(input);
            samples.scene.push(scene);
            samples.draw.push(draw);
            samples.total.push(total);
        }
    }

    let switches = renderer.dual_dispatch_switches_for_test() - starting_switches;
    assert!(samples.total.len() >= 55, "{label}: insufficient scroll frames");
    assert!(shift_frames >= 45, "{label}: scroll never reached Damage Tracker");
    assert!(full_frames >= 6, "{label}: no full invalidation crossover");
    assert!(small_frames >= 30, "{label}: no small single-rect damage");
    assert_eq!(checked, PARITY_STEPS.len() as u32);
    if mode == "adaptive2" {
        assert!(switches >= 8, "{label}: production adaptive context did not switch");
    } else {
        assert_eq!(switches, 0, "{label}: rollback unexpectedly routed");
    }

    let p = |values: &[f64]| percentiles(values, [0.5, 0.95]);
    let [total_p50, total_p95] = p(&samples.total);
    let [input_p50, input_p95] = p(&samples.input);
    let [scene_p50, scene_p95] = p(&samples.scene);
    let [draw_p50, draw_p95] = p(&samples.draw);
    let sum = |v: &[f64]| v.iter().sum::<f64>();
    let peak_kib = std::fs::read_to_string("/proc/self/status")
        .expect("Linux VmHWM")
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .expect("Linux peak process RSS in KiB");

    eprintln!(
        "AKULA_SCROLL_PIPELINE case={label} mode={mode} frames={} shifts={shift_frames} \
         full_frames={full_frames} small_frames={small_frames} switches={switches} \
         parity={checked} digest={digest:016x} peak_kib={peak_kib} \
         total_p50_us={total_p50:.2} total_p95_us={total_p95:.2} total_sum_us={:.2} \
         input_p50_us={input_p50:.2} input_p95_us={input_p95:.2} input_sum_us={:.2} \
         scene_p50_us={scene_p50:.2} scene_p95_us={scene_p95:.2} scene_sum_us={:.2} \
         draw_p50_us={draw_p50:.2} draw_p95_us={draw_p95:.2} draw_sum_us={:.2}",
        samples.total.len(),
        sum(&samples.total),
        sum(&samples.input),
        sum(&samples.scene),
        sum(&samples.draw),
    );

    // Join background workers before temporary vault cleanup.
    h.app.shutdown();
}

#[test]
#[ignore = "opt-in real pointer event to CPU frame pipeline A/B, not displayed FPS"]
fn benchmark_native_scroll_pipeline_ab() {
    let mode = match std::env::var("AKULA_VELLO_SINGLE_RECT").as_deref() {
        Ok("0") => "fixed2",
        Ok("1") => "adaptive2",
        other => panic!("explicit AKULA_VELLO_SINGLE_RECT=0 or 1 required: {other:?}"),
    };
    assert_ne!(
        std::env::var("AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT").ok().as_deref(),
        Some("0"),
        "legacy opt-out would silently disable the adaptive arm",
    );

    case(mode, "plain_1x", 1000, 600, 1.0, false);
    case(mode, "cover_175", 1750, 1051, 1.75, true);
}
