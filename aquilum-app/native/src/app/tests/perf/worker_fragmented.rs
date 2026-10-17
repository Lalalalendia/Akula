//! Opt-in fragmented damage regression: many small rectangles versus frame load.
//!
//! A large invalidation can be split into 2, 4, 8 or 16 strips. Switching
//! based on each strip's size then misclassifies the *aggregate* workload.
//! Two separate hot Vello contexts are retained by both routed variants.
//! This is a test, not a production policy.

use super::*;
use masonry::app::VisualLayerPlan;
use std::time::{Duration, Instant};

const FRAMES: u32 = 64;
const ROUTE_THRESHOLD_PX: u64 = 350_000;
const STRIP_COUNTS: [u16; 4] = [2, 4, 8, 16];

fn peak_channel(a: &[u32], b: &[u32]) -> u8 {
    assert_eq!(a.len(), b.len(), "framebuffer length");
    a.iter()
        .zip(b)
        .flat_map(|(&a, &b)| {
            [0, 8, 16, 24].map(move |shift| ((a >> shift) as u8).abs_diff((b >> shift) as u8))
        })
        .max()
        .unwrap_or(0)
}

fn draw(
    renderer: &mut Renderer,
    scene: &VisualLayerPlan,
    width: u16,
    height: u16,
    scale: f64,
    full: bool,
) -> f64 {
    let t = Instant::now();
    renderer
        .draw_frame(scene, width, height, scale, theme::background(), full)
        .unwrap();
    t.elapsed().as_secs_f64() * 1e6
}

fn fragmented_case(case: &str, width: u32, height: u32, scale: f64, cover: bool, full_every: u32, strips: u16) {
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)));
    h.app.window_resized(&mut h.root, Size::new(f64::from(width) / scale, f64::from(height) / scale));
    let text: String = (0..250)
        .map(|i| format!(
            "Абзац {i}: текст Akula, **жирный**, *курсив* и длинные Unicode \
             строки для переноса через Parley.\n\n"
        ))
        .collect();
    let frontmatter = if cover {
        "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n"
    } else {
        ""
    };
    let path = h.write_note("fragmented-damage.md", &format!("{frontmatter}{text}"));
    h.app.open(&mut h.root, path);
    if cover {
        h.wait_cover(true);
    }
    let _ = h.root.redraw();

    // Two fixed modes and two routing policies. All four use the *same*
    // disjoint fragmented full-frame damage rectangles.
    let mut renderers = [
        Renderer::new(0),
        Renderer::new(2),
        Renderer::new(2),
        Renderer::new(2),
    ];
    for renderer in &mut renderers {
        renderer.split_full_frame_for_test(strips);
    }
    renderers[2].enable_dual_dispatch_for_test(ROUTE_THRESHOLD_PX);
    renderers[3].enable_dual_dispatch_for_test(ROUTE_THRESHOLD_PX);
    renderers[3].enable_frame_budget_for_test();

    let (w, hh) = (width as u16, height as u16);
    let mut samples: [[Vec<f64>; 2]; 4] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
    let mut full_reference_checks = 0usize;

    for step in 0..FRAMES {
        if step % 8 == 7 {
            h.pointer_move(f64::from(70 + step * 8), f64::from(95 + step * 9));
        } else {
            let dy = if step < 40 { -29.0 } else { 23.0 };
            h.scroll((f64::from(width) * 0.6, f64::from(height) * 0.5), scale, (0.0, dy));
        }
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();
        let full = step == 0 || step % full_every == 0;

        // Rotate execution order without rebuilding renderers or resources.
        let order = match step % 4 {
            0 => [0, 1, 2, 3],
            1 => [1, 2, 3, 0],
            2 => [2, 3, 0, 1],
            _ => [3, 0, 1, 2],
        };
        let mut elapsed = [0.0; 4];
        for index in order {
            elapsed[index] = draw(&mut renderers[index], &layers, w, hh, scale, full);
        }

        for index in 1..4 {
            assert_eq!(
                renderers[0].frame_pixels(),
                renderers[index].frame_pixels(),
                "{case}/full_every={full_every}/step{step}: pixels differ for variant {index}"
            );
            assert_eq!(
                renderers[0].last_stages.raster_pixels,
                renderers[index].last_stages.raster_pixels,
                "{case}/step{step}: scheduled raster area differs"
            );
            assert_eq!(
                renderers[0].last_stages.raster_rects,
                renderers[index].last_stages.raster_rects,
                "{case}/step{step}: rectangle fragmentation differs"
            );
        }
        if full {
            let info = renderers[0].last_stages;
            assert_eq!(
                info.raster_rects,
                usize::from(strips),
                "{case}/step{step}: expected the fragmented full redraw"
            );
            assert_eq!(
                info.raster_pixels,
                u64::from(width) * u64::from(height),
                "{case}/step{step}: fragmented regions must cover the full screen"
            );
        }
        if matches!(step, 0 | 10 | 20 | 31 | 47 | 63) {
            // This independent renderer DOES NOT split its full framebuffer.
            let full_ref = render_full(&layers, w, hh, scale);
            let max = peak_channel(renderers[3].frame_pixels(), full_ref.frame_pixels());
            assert!(
                max <= 1,
                "{case}/step{step}: fragmented/full-reference channel delta {max}/255"
            );
            full_reference_checks += 1;
        }

        if step == 0 {
            continue;
        }
        let scrolling = renderers[0].last_stages.damage.shifts > 0;
        if !full && !scrolling {
            continue;
        }
        let class = usize::from(full);
        for index in 0..4 {
            samples[index][class].push(elapsed[index]);
        }
    }

    let per_rect_switches = renderers[2].dual_dispatch_switches_for_test();
    let frame_switches = renderers[3].dual_dispatch_switches_for_test();
    let largest_strip_pixels = u64::from(width) * u64::from((height + u32::from(strips) - 1) / u32::from(strips));
    if largest_strip_pixels <= ROUTE_THRESHOLD_PX {
        assert!(per_rect_switches <= 2, "{case}/strips={strips}: per-rect route unexpectedly switched {per_rect_switches} times");
    } else {
        assert!(per_rect_switches >= 6, "{case}/strips={strips}: per-rect route should switch on large strips: {per_rect_switches}");
    }
    assert!(frame_switches >= 6, "{case}: aggregate router did not switch modes: {frame_switches}");
    assert_eq!(full_reference_checks, 6);

    for (index, mode) in ["serial", "parallel2", "per_rect", "frame_budget"].into_iter().enumerate() {
        for (class, workload) in ["scroll", "full"].into_iter().enumerate() {
            let values = &samples[index][class];
            assert!(values.len() >= 4, "{case}/{mode}/{workload}: insufficient samples");
            let [p50, p95] = percentiles(values, [0.5, 0.95]);
            let total: f64 = values.iter().sum();
            eprintln!(
                "VELLO_FRAGMENTED_ROUTE case={case} full_every={full_every} mode={mode} \
                 workload={workload} frames={} strips={strips} per_rect_switches={per_rect_switches} \
                 frame_switches={frame_switches} p50_us={p50:.2} p95_us={p95:.2} total_us={total:.2}",
                values.len()
            );
        }
    }
}

#[test]
#[ignore = "opt-in fragmented real frame route; production renderer unchanged"]
fn benchmark_vello_fragmented_route() {
    for (case, width, height, scale, cover) in [
        ("text_1x", 1000, 600, 1.0, false),
        ("text_175", 1750, 1051, 1.75, false),
        ("cover_175", 1750, 1051, 1.75, true),
    ] {
        for full_every in [8, 3] {
            for strips in STRIP_COUNTS {
                fragmented_case(case, width, height, scale, cover, full_every, strips);
            }
        }
    }
}
