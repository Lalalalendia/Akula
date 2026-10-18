//! Opt-in, release-mode crossover study of 0, 2 and 4 raster workers.
//!
//! Uses the same VisualLayerPlan for each worker mode, rotates draw order and
//! measures only frames which actually scroll. No production routing changes.

use super::*;
use masonry::app::VisualLayerPlan;
use std::time::{Duration, Instant};

const STEPS: u32 = 32;
const WORKERS: [u16; 3] = [0, 2, 4];

#[derive(Clone, Copy)]
struct WorkerSample {
    total_us: f64,
    encode_us: f64,
    raster_us: f64,
    raster_pct: f64,
    scroll_candidates: usize,
}

fn draw_worker(
    renderer: &mut Renderer,
    layers: &VisualLayerPlan,
    width: u16,
    height: u16,
    scale: f64,
    full: bool,
) -> WorkerSample {
    let started = Instant::now();
    renderer
        .draw_frame(layers, width, height, scale, theme::background(), full)
        .expect("worker benchmark frame");
    let stages = renderer.last_stages;
    WorkerSample {
        total_us: started.elapsed().as_secs_f64() * 1e6,
        encode_us: stages.encode.as_secs_f64() * 1e6,
        raster_us: stages.raster.as_secs_f64() * 1e6,
        raster_pct: 100.0 * stages.raster_pixels as f64 / stages.screen_pixels as f64,
        scroll_candidates: stages.damage.shifts,
    }
}

fn max_pixel_channel_delta(a: &[u32], b: &[u32]) -> u8 {
    assert_eq!(a.len(), b.len(), "different frame dimensions");
    a.iter()
        .zip(b)
        .flat_map(|(&a, &b)| {
            [0, 8, 16, 24]
                .map(move |shift| ((a >> shift) as u8).abs_diff((b >> shift) as u8))
        })
        .max()
        .unwrap_or(0)
}

// Only on a *real* strict parity failure: report the first bad pixels and
// optionally retain both PNGs for investigation. No changed test tolerance.
fn diagnose_channel_mismatch(
    case: &str,
    workload: &str,
    step: u32,
    width: u16,
    partial: &Renderer,
    full: &Renderer,
    previous_full: Option<&[u32]>,
    previous_partial: Option<&[u32]>,
) {
    let (mut count, mut worst) = (0usize, 0u8);
    let mut examples = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) =
        (usize::MAX, usize::MAX, 0usize, 0usize);
    for (pixel, (&a, &b)) in partial.frame_pixels().iter().zip(full.frame_pixels()).enumerate() {
        let peak = [0, 8, 16, 24].into_iter().map(|shift| {
            ((a >> shift) as u8).abs_diff((b >> shift) as u8)
        }).max().unwrap_or(0);
        if peak <= 1 { continue; }
        count += 1;
        worst = worst.max(peak);
        let (x, y) = (pixel % usize::from(width), pixel / usize::from(width));
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
        if examples.len() < 16 {
            let shifted_from = partial.moved.iter().find_map(|&(mx, my, mw, mh, dy)| {
                let within_x = x >= usize::from(mx) && x < usize::from(mx) + usize::from(mw);
                let within_y = y >= usize::from(my) && y < usize::from(my) + usize::from(mh);
                if !within_x || !within_y {
                    return None;
                }
                let source_y = usize::try_from(i64::try_from(y).ok()? - i64::from(dy)).ok()?;
                let offset = source_y.checked_mul(usize::from(width))?.checked_add(x)?;
                let prior_full_color = previous_full
                    .and_then(|pixels| pixels.get(offset))
                    .map(|color| format!("{color:08x}"));
                let prior_partial_color = previous_partial
                    .and_then(|pixels| pixels.get(offset))
                    .map(|color| format!("{color:08x}"));
                let prior_partial_vs_full = previous_partial
                    .and_then(|pixels| pixels.get(offset))
                    .zip(previous_full.and_then(|pixels| pixels.get(offset)))
                    .map(|(&p, &f)| [0, 8, 16, 24].into_iter()
                        .map(|bits| ((p >> bits) as u8).abs_diff((f >> bits) as u8))
                        .max().unwrap_or(0));
                Some((source_y, prior_full_color, prior_partial_color, prior_partial_vs_full))
            });
            let rerastered = partial.rastered.iter().any(|&(rx, ry, rw, rh)| {
                x >= usize::from(rx) && x < usize::from(rx) + usize::from(rw)
                    && y >= usize::from(ry) && y < usize::from(ry) + usize::from(rh)
            });
            examples.push(format!(
                "({x},{y}):{a:08x}/{b:08x} peak={peak} shifted_from={shifted_from:?} rerastered={rerastered}"
            ));
        }
    }
    eprintln!(
        "VELLO_GUARD_PARITY_FAILURE case={case} workload={workload} step={step} \
         mismatched_gt1={count} max_channel={worst} bbox={x0},{y0},{x1},{y1} \
         moved={:?} rastered={:?} examples={examples:?}",
        partial.moved, partial.rastered,
    );
    if let Some(output) = std::env::var_os("AQ_CROSSOVER_OUT") {
        let root = std::path::PathBuf::from(output);
        std::fs::create_dir_all(&root).expect("create parity artifact directory");
        std::fs::write(root.join(format!("{case}-{workload}-step{step}-partial.png")), partial.png())
            .expect("write partial parity image");
        std::fs::write(root.join(format!("{case}-{workload}-step{step}-full.png")), full.png())
            .expect("write full parity image");
    }
}

fn median_and_p95(samples: &[WorkerSample], select: impl Fn(&WorkerSample) -> f64) -> [f64; 2] {
    let values: Vec<f64> = samples.iter().map(select).collect();
    percentiles(&values, [0.50, 0.95])
}

// The tracker reports scroll *candidates* before test-only redraw
// overrides are applied. In viewport/full modes no row copy should occur.
fn assert_selected_damage_mode(workload: &str, case: &str, step: u32, area_pct: f64, actual_moves: usize) {
    let (min_pct, max_pct, expect_moves) = match workload {
        "normal" => (0.5, 5.0, true),
        "guard96" => (5.0, 20.0, true),
        "viewport" => (20.0, 50.0, false),
        "full" => (99.0, 101.0, false),
        other => panic!("unknown Vello workload {other}"),
    };
    assert!(
        area_pct >= min_pct && area_pct <= max_pct,
        "{case}/{workload}/frame{step}: wrong raster workload {area_pct:.2}% (expected {min_pct:.1}–{max_pct:.1}%)",
    );
    assert_eq!(
        actual_moves > 0,
        expect_moves,
        "{case}/{workload}/frame{step}: unexpected physical row-copy behavior (moves={actual_moves})",
    );
}

fn run_worker_case(case: &str, width: u32, height: u32, scale: f64, cover: bool, workload: &str) {
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
    let fm = if cover {
        "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n"
    } else {
        ""
    };
    let path = h.write_note("worker-crossover.md", &format!("{fm}{body}"));
    h.app.open(&mut h.root, path);
    if cover {
        h.wait_cover(true);
    }
    let _ = h.root.redraw();
    let (w, hh) = (width as u16, height as u16);
    let mut renderers = [
        Renderer::new(WORKERS[0]),
        Renderer::new(WORKERS[1]),
        Renderer::new(WORKERS[2]),
    ];
    let mut samples: [Vec<WorkerSample>; 3] =
        std::array::from_fn(|_| Vec::with_capacity((STEPS - 1) as usize));
    let mut previous_full: Option<(u32, Vec<u32>)> = None;
    let mut previous_partial: Option<(u32, Vec<u32>)> = None;

    for step in 0..STEPS {
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

        // Rotate ABC/BCA/CAB so the slowest renderer is not always last.
        let order = match step % 3 {
            0 => [0, 1, 2],
            1 => [1, 2, 0],
            _ => [2, 0, 1],
        };
        for index in order {
            if std::env::var_os("AQ_TRACE_ATLAS_PIXEL").is_some() {
                eprintln!(
                    "VELLO_ATLAS_PASS case={case} workload={workload} step={step}                      kind=partial workers={}",
                    WORKERS[index],
                );
            }
            let sample = draw_worker(&mut renderers[index], &layers, w, hh, scale, step == 0);
            // Sample scroll-candidate frames only. The tracker records the
            // candidate before the viewport/full test mode suppresses copies.
            // Verify each mode actually exercises its intended raster area.
            if step != 0 && sample.scroll_candidates > 0 {
                assert_selected_damage_mode(workload, case, step, sample.raster_pct, renderers[index].moved.len());
                samples[index].push(sample);
            }
        }

        for index in 1..renderers.len() {
            assert_eq!(
                renderers[0].frame_pixels(),
                renderers[index].frame_pixels(),
                "{case}/{workload}: 0 vs {} workers differ on frame {step}",
                WORKERS[index],
            );
        }
        if matches!(step, 0 | 10 | 20 | 31) || std::env::var_os("AQ_CROSSOVER_EVERY_FRAME").is_some() {
            if std::env::var_os("AQ_TRACE_ATLAS_PIXEL").is_some() {
                eprintln!("VELLO_ATLAS_PASS case={case} workload={workload} step={step} kind=full");
            }
            let full = render_full(&layers, w, hh, scale);
            let peak = max_pixel_channel_delta(renderers[0].frame_pixels(), full.frame_pixels());
            if peak > 1 {
                let previous = previous_full.as_ref().and_then(|(previous_step, pixels)| {
                    (*previous_step + 1 == step).then_some(pixels.as_slice())
                });
                let previous_part = previous_partial.as_ref().and_then(|(previous_step, pixels)| {
                    (*previous_step + 1 == step).then_some(pixels.as_slice())
                });
                diagnose_channel_mismatch(
                    case, workload, step, w, &renderers[0], &full, previous, previous_part,
                );
                if std::env::var_os("AQ_TRACE_ATLAS_PIXEL").is_some() {
                    eprintln!(
                        "VELLO_ATLAS_PASS case={case} workload={workload} step={step} kind=full_repeat"
                    );
                }
                let repeated = render_full(&layers, w, hh, scale);
                let repeated_peak =
                    max_pixel_channel_delta(full.frame_pixels(), repeated.frame_pixels());
                eprintln!(
                    "VELLO_GUARD_FULL_REPEAT case={case} workload={workload} step={step} max_channel={repeated_peak}"
                );
            }
            previous_full = Some((step, full.frame_pixels().to_vec()));
            assert!(
                peak <= 1,
                "{case}/{workload}: partial/full channel difference {peak}/255 on frame {step}",
            );
        }
        // Preserve the actually displayed partial result, not just its fresh
        // reference: each scroll shift copies the prior partial framebuffer.
        previous_partial = Some((step, renderers[0].frame_pixels().to_vec()));
    }

    for (index, thread_count) in WORKERS.into_iter().enumerate() {
        let selected = &samples[index];
        assert!(
            selected.len() >= 20,
            "{case}/{workload}/{thread_count}: too few scroll-candidate frames ({})",
            selected.len()
        );
        let [total_p50, total_p95] = median_and_p95(selected, |s| s.total_us);
        let [encode_p50, encode_p95] = median_and_p95(selected, |s| s.encode_us);
        let [raster_p50, raster_p95] = median_and_p95(selected, |s| s.raster_us);
        let [area_p50, area_p95] = median_and_p95(selected, |s| s.raster_pct);
        eprintln!(
            "VELLO_WORKER_CROSSOVER case={case} workload={workload} workers={thread_count} \
             frames={} total_p50_us={total_p50:.2} total_p95_us={total_p95:.2} \
             encode_p50_us={encode_p50:.2} encode_p95_us={encode_p95:.2} \
             raster_p50_us={raster_p50:.2} raster_p95_us={raster_p95:.2} \
             raster_area_p50_pct={area_p50:.3} raster_area_p95_pct={area_p95:.3}",
            selected.len(),
        );
    }
}

#[test]
#[ignore = "opt-in 0/2/4-worker release-mode crossover; production policy unchanged"]
fn benchmark_vello_worker_crossover() {
    let workload = std::env::var("AQ_CROSSOVER_WORKLOAD").unwrap_or_else(|_| "normal".into());
    let only = std::env::var("AQ_CROSSOVER_ONLY_CASE").ok();
    let mut executed = 0usize;
    for (case, width, height, scale, cover) in [
        ("text_1x", 1000, 600, 1.0, false),
        ("text_175", 1750, 1051, 1.75, false),
        ("cover_1x", 1000, 600, 1.0, true),
        ("cover_175", 1750, 1051, 1.75, true),
    ] {
        if let Some(ref selected) = only {
            if selected != case { continue; }
        }
        executed += 1;
        run_worker_case(case, width, height, scale, cover, &workload);
    }
    assert!(executed != 0, "AQ_CROSSOVER_ONLY_CASE selected no known scene: {only:?}");
}
