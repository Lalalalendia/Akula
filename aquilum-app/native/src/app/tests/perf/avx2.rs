//! Test-only comparison of the real Akula editor frames under SSE4.2 and AVX2.
//! No production SIMD selection is changed by these tests.

use super::*;
use std::time::{Duration, Instant};

fn detected_pair() -> Option<(vello_cpu::Level, vello_cpu::Level)> {
    let available = vello_cpu::RenderSettings::default().level;
    Some((
        vello_cpu::Level::Sse4_2(available.as_sse4_2()?),
        vello_cpu::Level::Avx2(available.as_avx2()?),
    ))
}

fn assert_identical(label: &str, width: u16, old: &[u32], new: &[u32]) {
    assert_eq!(old.len(), new.len(), "{label}: frame size differs");
    let mut unequal = 0usize;
    let mut max_channel_delta = 0u8;
    let mut within_two = 0usize;
    let mut bbox = (usize::MAX, usize::MAX, 0usize, 0usize);
    let mut examples = Vec::new();
    let mut regions = [0usize; 4];
    for (index, (&a, &b)) in old.iter().zip(new).enumerate() {
        if a == b {
            continue;
        }
        unequal += 1;
        let x = index % usize::from(width);
        let y = index / usize::from(width);
        bbox.0 = bbox.0.min(x);
        bbox.1 = bbox.1.min(y);
        bbox.2 = bbox.2.max(x);
        bbox.3 = bbox.3.max(y);
        regions[usize::from(x >= usize::from(width) / 2) + 2 * usize::from(y >= old.len() / usize::from(width) / 2)] += 1;
        let delta = [0, 8, 16, 24].iter().map(|shift| {
            ((a >> shift) as u8).abs_diff((b >> shift) as u8)
        }).max().unwrap_or_default();
        if delta <= 2 {
            within_two += 1;
        }
        max_channel_delta = max_channel_delta.max(delta);
        if examples.len() < 16 {
            examples.push(format!("({x},{y}): {a:08x} vs {b:08x} delta={delta}"));
        }
    }
    if unequal != 0 {
        eprintln!("DIAG-AVX2-DIFF case={label:?} size={width}x{} changed={unequal} max_channel_delta={max_channel_delta} within_two={within_two} bbox={bbox:?} quadrants={regions:?} examples={examples:?}", old.len() / usize::from(width));
    }
    assert_eq!(unequal, 0, "{label}: SSE4.2 and AVX2 differ in {unequal} pixels out of {}", old.len());
}

fn measured_draw(
    renderer: &mut Renderer,
    layers: &masonry::app::VisualLayerPlan,
    width: u16,
    height: u16,
    scale: f64,
    full: bool,
) -> (f64, usize) {
    let start = Instant::now();
    let areas = renderer.draw_frame(layers, width, height, scale, theme::background(), full).unwrap();
    (start.elapsed().as_secs_f64() * 1000.0, areas.len())
}

fn median(mut data: Vec<f64>) -> f64 {
    data.sort_by(f64::total_cmp);
    (data[data.len() / 2 - 1] + data[data.len() / 2]) / 2.0
}

fn scroll_parity(scale: f64, width: u32, height: u32, threads: u16) {
    let Some((sse_level, avx_level)) = detected_pair() else {
        eprintln!("SKIP-AVX2-INTEGRATED unsupported CPU");
        return;
    };

    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)));
    h.app.window_resized(&mut h.root, Size::new(f64::from(width) / scale, f64::from(height) / scale));
    let body: String = (0..240)
        .map(|i| format!("Абзац {i}: Unicode 😀 и кириллица, подробный текст документа Akula с **жирным** и *курсивом*.\n\n"))
        .collect();
    let note = h.write_note(
        "AVX2 экран.md",
        &format!("---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n{body}"),
    );
    h.app.open(&mut h.root, note);
    h.wait_cover(true);
    let (w, hh) = (width as u16, height as u16);
    let mut baseline = Renderer::with_simd_level_for_test(threads, sse_level);
    let mut candidate = Renderer::with_simd_level_for_test(threads, avx_level);
    let (mut before, mut after) = (Vec::new(), Vec::new());
    let (mut baseline_damage, mut candidate_damage) = (0, 0);

    // A/B order is balanced over successive real UI frames, and both
    // renderers see exactly the same VisualLayerPlan at every step.
    for step in 0..36 {
        if step % 7 == 6 {
            h.pointer_move(f64::from(40 + step * 7), f64::from(80 + step * 11));
        } else {
            let dy = if step < 22 { -29.0 } else { 19.0 };
            h.scroll((f64::from(width) * 0.6, f64::from(height) * 0.5), scale, (0.0, dy));
        }
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();

        let first_sse = matches!(step % 4, 0 | 3);
        let ((b, db), (a, da)) = if first_sse {
            (
                measured_draw(&mut baseline, &layers, w, hh, scale, step == 0),
                measured_draw(&mut candidate, &layers, w, hh, scale, step == 0),
            )
        } else {
            let a = measured_draw(&mut candidate, &layers, w, hh, scale, step == 0);
            let b = measured_draw(&mut baseline, &layers, w, hh, scale, step == 0);
            (b, a)
        };
        before.push(b);
        after.push(a);
        baseline_damage += db;
        candidate_damage += da;
        if baseline.frame_pixels() != candidate.frame_pixels() && step == 0 {
            if let Ok(dir) = std::env::var("AQ_OUT") {
                let out = std::path::Path::new(&dir);
                std::fs::create_dir_all(out).unwrap();
                std::fs::write(out.join(format!("avx2-scroll-{scale}-sse.png")), baseline.png()).unwrap();
                std::fs::write(out.join(format!("avx2-scroll-{scale}-avx.png")), candidate.png()).unwrap();
            }
            for probe_threads in [0u16, 2] {
                let mut probe_sse = Renderer::with_simd_level_for_test(probe_threads, sse_level);
                let mut probe_avx = Renderer::with_simd_level_for_test(probe_threads, avx_level);
                probe_sse.draw_frame(&layers, w, hh, scale, theme::background(), true).unwrap();
                probe_avx.draw_frame(&layers, w, hh, scale, theme::background(), true).unwrap();
                let mismatch = probe_sse.frame_pixels().iter().zip(probe_avx.frame_pixels()).filter(|(a,b)| a != b).count();
                eprintln!("DIAG-AVX2-SINGLE-RENDER scale={scale} threads={probe_threads} pixel_diff={mismatch}");
            }
        }
        assert_identical(&format!("scroll {scale} threads={threads} step={step}"), w, baseline.frame_pixels(), candidate.frame_pixels());
        if step % 6 == 0 {
            let full = render_full(&layers, w, hh, scale);
            assert_eq!(differing_pixels(baseline.frame_pixels(), full.frame_pixels()), 0,
                "step={step}: existing SSE partial frame diverged from full rendering");
            assert_eq!(differing_pixels(candidate.frame_pixels(), full.frame_pixels()), 0,
                "step={step}: candidate AVX2 partial frame diverged from full rendering");
        }
    }
    assert!(baseline.shifts > 16, "test fixture did not exercise pixel scroll shifts");
    assert_eq!(baseline_damage, candidate_damage, "damage geometry differs between SIMD levels");
    let sse_ms = median(before);
    let avx_ms = median(after);
    eprintln!(
        "BENCH-AVX2-EDITOR case=scroll scale={scale} threads={threads} sse_ms={sse_ms:.4} avx2_ms={avx_ms:.4} ratio={:.3}",
        sse_ms / avx_ms
    );
    eprintln!("PASS-AVX2-INTEGRATED scroll scale={scale} threads={threads} steps=36 pixel_diff=0");
}

#[test]
#[ignore = "integration parity on x86 AVX2-capable runner"]
fn avx2_integrated_scroll_standard() {
    scroll_parity(1.0, W, H, 2);
}

#[test]
#[ignore = "integration parity on x86 AVX2-capable runner"]
fn avx2_integrated_scroll_fractional() {
    scroll_parity(1.75, 1750, 1051, 2);
}

struct LiveFreezeGuard;
impl Drop for LiveFreezeGuard {
    fn drop(&mut self) {
        crate::render::live::freeze(None);
    }
}

#[test]
#[ignore = "animated cover and partial-live pixel parity on x86"]
fn avx2_integrated_live_cover() {
    let Some((sse_level, avx_level)) = detected_pair() else {
        eprintln!("SKIP-AVX2-INTEGRATED unsupported CPU");
        return;
    };
    let mut h = Harness::new();
    let note = h.write_note("Animated AVX2.md",
        "---\ncover: true\nPage_cover: tunnel\n---\nТекст под живой обложкой.");
    h.app.open(&mut h.root, note);
    h.wait_cover(true);
    h.anim(Duration::from_secs(1));
    let (layers, _) = h.root.redraw();
    let mut baseline = Renderer::with_simd_level_for_test(2, sse_level);
    let mut candidate = Renderer::with_simd_level_for_test(2, avx_level);
    let _freeze_guard = LiveFreezeGuard;
    crate::render::live::freeze(Some(0.0));
    baseline.draw_frame(&layers, W as u16, H as u16, 1.0, theme::background(), true).unwrap();
    candidate.draw_frame(&layers, W as u16, H as u16, 1.0, theme::background(), true).unwrap();
    assert!(baseline.has_live() && candidate.has_live(), "fixture must contain animated cover");
    assert_identical("cover first frame", W as u16, baseline.frame_pixels(), candidate.frame_pixels());
    let mut previous = baseline.frame_pixels().to_vec();
    let mut changed_frames = 0;
    for step in 1..=8 {
        crate::render::live::freeze(Some(f64::from(step) * 0.137));
        let touched_sse = baseline.live_frame(&layers).unwrap();
        let touched_avx = candidate.live_frame(&layers).unwrap();
        assert_eq!(touched_sse, touched_avx, "live damage regions mismatch at step {step}");
        assert!(!touched_sse.is_empty(), "live animation produced no damage on step {step}");
        assert_identical(&format!("cover step={step}"), W as u16, baseline.frame_pixels(), candidate.frame_pixels());
        let full = render_full(&layers, W as u16, H as u16, 1.0);
        assert_eq!(differing_pixels(candidate.frame_pixels(), full.frame_pixels()), 0,
            "AVX2 live cover diverged from full redraw at step {step}");
        if differing_pixels(&previous, baseline.frame_pixels()) > 100 {
            changed_frames += 1;
        }
        previous.copy_from_slice(baseline.frame_pixels());
    }
    assert!(changed_frames >= 4, "animated cover was not changing across the test");
    eprintln!("PASS-AVX2-INTEGRATED animated_cover steps=8 pixel_diff=0");
}
