//! Test-only A/B of exact-coverage adjacent-rectangle coalescing.
//!
//! We compare four persistent renderers on identical Akula editor frames:
//! unchanged 0/2-worker rectangle loops and the exact same workers after
//! coalescing only disjoint, full-edge neighbours. No bounding-box expansion.
//! The breakdown isolates reset, scene replay, flush and fine raster cost.

use super::*;
use masonry::app::VisualLayerPlan;
use std::time::{Duration, Instant};

const FRAMES: u32 = 64;
const STRIPS: [u16; 4] = [2, 4, 8, 16];
const MODES: [&str; 4] = ["serial", "parallel2", "coalesced0", "coalesced2"];

#[derive(Clone, Copy)]
struct Timing {
    total: f64,
    reset: f64,
    replay: f64,
    flush: f64,
    raster: f64,
    rects: usize,
    pixels: u64,
}

fn draw_batch(
    renderer: &mut Renderer,
    scene: &VisualLayerPlan,
    width: u16,
    height: u16,
    scale: f64,
    full: bool,
) -> Timing {
    let start = Instant::now();
    renderer
        .draw_frame(scene, width, height, scale, theme::background(), full)
        .expect("adjacent-region A/B frame");
    let total = start.elapsed().as_secs_f64() * 1e6;
    let stages = renderer.last_stages;
    Timing {
        total,
        reset: stages.reset.as_secs_f64() * 1e6,
        replay: stages.replay.as_secs_f64() * 1e6,
        flush: stages.flush.as_secs_f64() * 1e6,
        raster: stages.raster.as_secs_f64() * 1e6,
        rects: stages.raster_rects,
        pixels: stages.raster_pixels,
    }
}

fn peak_channel(a: &[u32], b: &[u32]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .flat_map(|(&x, &y)| {
            [0, 8, 16, 24].map(move |shift| {
                ((x >> shift) as u8).abs_diff((y >> shift) as u8)
            })
        })
        .max()
        .unwrap_or(0)
}

fn quantiles(times: &[Timing], get: impl Fn(&Timing) -> f64) -> [f64; 2] {
    let data: Vec<f64> = times.iter().map(get).collect();
    percentiles(&data, [0.50, 0.95])
}

fn run_batch_case(
    case: &str,
    width: u32,
    height: u32,
    scale: f64,
    cover: bool,
    full_every: u32,
    strips: u16,
) {
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(
        masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)),
    );
    h.app.window_resized(
        &mut h.root,
        Size::new(f64::from(width) / scale, f64::from(height) / scale),
    );
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
    let path = h.write_note("adjacent-batch.md", &format!("{frontmatter}{text}"));
    h.app.open(&mut h.root, path);
    if cover {
        h.wait_cover(true);
    }
    let _ = h.root.redraw();

    let (w, hh) = (width as u16, height as u16);
    let mut renderers = [
        Renderer::new(0),
        Renderer::new(2),
        Renderer::new(0),
        Renderer::new(2),
    ];
    for renderer in &mut renderers {
        renderer.split_full_frame_for_test(strips);
    }
    renderers[2].enable_adjacent_coalescing_for_test();
    renderers[3].enable_adjacent_coalescing_for_test();
    let mut samples: [[Vec<Timing>; 2]; 4] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
    let mut checks = 0usize;

    for step in 0..FRAMES {
        if step % 8 == 7 {
            h.pointer_move(f64::from(70 + step * 8), f64::from(95 + step * 9));
        } else {
            let dy = if step < 40 { -29.0 } else { 23.0 };
            h.scroll(
                (f64::from(width) * 0.6, f64::from(height) * 0.5),
                scale,
                (0.0, dy),
            );
        }
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();
        let full = step == 0 || step % full_every == 0;

        let order = match step % 4 {
            0 => [0, 1, 2, 3],
            1 => [1, 2, 3, 0],
            2 => [2, 3, 0, 1],
            _ => [3, 0, 1, 2],
        };
        let mut frame: [Option<Timing>; 4] = [None; 4];
        for index in order {
            frame[index] = Some(draw_batch(
                &mut renderers[index],
                &layers,
                w,
                hh,
                scale,
                full,
            ));
        }
        let times = frame.map(|v| v.expect("all four renderers drew this frame"));

        for index in 1..4 {
            assert_eq!(
                renderers[0].frame_pixels(),
                renderers[index].frame_pixels(),
                "{case}/strips={strips}/step={step}/mode={}: byte parity",
                MODES[index],
            );
            assert_eq!(
                times[0].pixels,
                times[index].pixels,
                "{case}/strips={strips}/step={step}: extra pixels rendered",
            );
            assert!(
                times[index].rects <= times[0].rects,
                "{case}/step={step}: coalescing created more rectangles",
            );
        }
        if full {
            assert_eq!(
                times[0].rects,
                usize::from(strips),
                "{case}/step={step}: input must be exactly {strips} bands"
            );
            assert_eq!(
                times[1].rects,
                usize::from(strips),
                "{case}/step={step}: parallel input differs"
            );
            assert_eq!(
                times[2].rects, 1,
                "{case}/step={step}: serial should coalesce to exactly 1 region"
            );
            assert_eq!(
                times[3].rects, 1,
                "{case}/step={step}: parallel should coalesce to exactly 1 region"
            );
            assert_eq!(
                times[0].pixels,
                u64::from(width) * u64::from(height),
                "{case}/step={step}: expected exactly full-frame pixel area"
            );
        }
        if matches!(step, 0 | 10 | 20 | 31 | 47 | 63) {
            let reference = render_full(&layers, w, hh, scale);
            for renderer in &renderers {
                let peak = peak_channel(renderer.frame_pixels(), reference.frame_pixels());
                assert!(
                    peak <= 1,
                    "{case}/step={step}/strips={strips}: cropped-vs-full delta {peak}/255"
                );
            }
            checks += 1;
        }

        if step == 0 {
            continue;
        }
        let has_scroll = renderers[0].last_stages.damage.shifts > 0;
        if !full && !has_scroll {
            continue;
        }
        let class = usize::from(full);
        for index in 0..4 {
            samples[index][class].push(times[index]);
        }
    }
    assert_eq!(checks, 6);

    for (index, mode) in MODES.iter().enumerate() {
        for (class, workload) in ["scroll", "full"].into_iter().enumerate() {
            let values = &samples[index][class];
            assert!(
                values.len() >= 4,
                "{case}/strips={strips}/{mode}/{workload}: too few samples"
            );
            let [p50, p95] = quantiles(values, |v| v.total);
            let [reset, _] = quantiles(values, |v| v.reset);
            let [replay, _] = quantiles(values, |v| v.replay);
            let [flush, _] = quantiles(values, |v| v.flush);
            let [raster, _] = quantiles(values, |v| v.raster);
            let sum: f64 = values.iter().map(|v| v.total).sum();
            let reset_sum: f64 = values.iter().map(|v| v.reset).sum();
            let replay_sum: f64 = values.iter().map(|v| v.replay).sum();
            let flush_sum: f64 = values.iter().map(|v| v.flush).sum();
            let raster_sum: f64 = values.iter().map(|v| v.raster).sum();
            let rects = values.iter().map(|v| v.rects).max().unwrap_or(0);
            eprintln!(
                "VELLO_ADJACENT_AB case={case} strips={strips} full_every={full_every} \
                 mode={mode} workload={workload} frames={} rects_max={rects} \
                 total_p50_us={p50:.2} total_p95_us={p95:.2} \
                 reset_p50_us={reset:.2} replay_p50_us={replay:.2} \
                 flush_p50_us={flush:.2} raster_p50_us={raster:.2} \
                 total_sum_us={sum:.2} reset_sum_us={reset_sum:.2} \
                 replay_sum_us={replay_sum:.2} flush_sum_us={flush_sum:.2} \
                 raster_sum_us={raster_sum:.2}",
                values.len()
            );
        }
    }
}

#[test]
#[ignore = "opt-in lossless adjacent-region batching A/B; production unchanged"]
fn benchmark_vello_adjacent_batching() {
    for (case, width, height, scale, cover) in [
        ("text_1x", 1000, 600, 1.0, false),
        ("text_175", 1750, 1051, 1.75, false),
        ("cover_175", 1750, 1051, 1.75, true),
    ] {
        for full_every in [8, 3] {
            for strips in STRIPS {
                run_batch_case(
                    case, width, height, scale, cover, full_every, strips,
                );
            }
        }
    }
}
