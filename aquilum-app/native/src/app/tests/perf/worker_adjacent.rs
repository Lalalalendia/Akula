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

/// Diagnostic with bounded output; the default assert_eq! for million-pixel
/// framebuffers would flood CI logs and obscure which pixels actually differ.
fn describe_diff(
    case: &str,
    step: u32,
    strips: u16,
    mode: &str,
    width: u16,
    a: &[u32],
    b: &[u32],
) -> (usize, usize, u8) {
    assert_eq!(a.len(), b.len(), "frame dimensions differ");
    let mut changed = 0usize;
    let mut worse_than_one = 0usize;
    let mut peak = 0u8;
    let mut bbox = (usize::MAX, usize::MAX, 0usize, 0usize);
    let mut examples = Vec::new();
    let mut at_seam = 0usize;
    for (index, (&left, &right)) in a.iter().zip(b).enumerate() {
        if left == right { continue; }
        let px = index % usize::from(width);
        let py = index / usize::from(width);
        let delta = [0, 8, 16, 24].map(|bits| {
            ((left >> bits) as u8).abs_diff((right >> bits) as u8)
        }).into_iter().max().unwrap_or(0);
        changed += 1;
        worse_than_one += usize::from(delta > 1);
        peak = peak.max(delta);
        bbox.0 = bbox.0.min(px);
        bbox.1 = bbox.1.min(py);
        bbox.2 = bbox.2.max(px);
        bbox.3 = bbox.3.max(py);
        let height = a.len() / usize::from(width);
        let band = height.div_ceil(usize::from(strips));
        if band != 0 && (py % band < 2 || (py + 1) % band < 2) {
            at_seam += 1;
        }
        if examples.len() < 16 {
            examples.push(format!("({px},{py}):{left:08x}/{right:08x} delta={delta}"));
        }
    }
    if changed > 0 {
        eprintln!(
            "VELLO_ADJACENT_DIFF case={case} strips={strips} step={step} mode={mode} \
             pixels_changed={changed} worse_than_one={worse_than_one} max_channel={peak} \
             seam_near_pixels={at_seam} bbox={bbox:?} examples={examples:?}"
        );
    }
    (changed, worse_than_one, peak)
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

        // Same-region-plan serial/parallel paths must be byte-identical.
        // Changing crop geometry can legitimately change at most one 8-bit
        // channel step; the existing Akula full-redraw gate is <=1/255.
        // Never use the looser threshold for identical partitioning.
        for (left, right, exact) in [
            (0usize, 1usize, true),
            (2, 3, true),
            (0, 2, false),
            (0, 3, false),
        ] {
            let (changed, severe, peak) = describe_diff(
                case, step, strips, MODES[right], w,
                renderers[left].frame_pixels(), renderers[right].frame_pixels(),
            );
            if severe > 0 || (exact && changed > 0) {
                let reference = render_full(&layers, w, hh, scale);
                describe_diff(
                    case, step, strips, "baseline_vs_full", w,
                    renderers[0].frame_pixels(), reference.frame_pixels(),
                );
                describe_diff(
                    case, step, strips, "variant_vs_full", w,
                    renderers[right].frame_pixels(), reference.frame_pixels(),
                );
                if let Some(path) = std::env::var_os("AQ_ADJACENT_DIAG_OUT") {
                    let dir = std::path::PathBuf::from(path);
                    std::fs::create_dir_all(&dir).expect("create diagnostic artifact directory");
                    let stem = format!("{case}-strips{strips}-step{step}");
                    std::fs::write(dir.join(format!("{stem}-baseline.png")), renderers[0].png()).expect("baseline png");
                    std::fs::write(dir.join(format!("{stem}-{}.png", MODES[right])), renderers[right].png()).expect("variant png");
                    std::fs::write(dir.join(format!("{stem}-full.png")), reference.png()).expect("reference png");
                }
            }
            assert!(
                severe == 0 && (!exact || changed == 0),
                "{case}/strips={strips}/step={step}: {} vs {} \
                 exact={exact} changed={changed} severe_gt1={severe} peak={peak}/255",
                MODES[left], MODES[right],
            );
        }
        for index in 1..4 {
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
    let only_case = std::env::var("AQ_ADJACENT_ONLY_CASE").ok();
    let only_strips = std::env::var("AQ_ADJACENT_ONLY_STRIPS").ok().map(|v| v.parse::<u16>().expect("valid strip count"));
    let only_every = std::env::var("AQ_ADJACENT_ONLY_EVERY").ok().map(|v| v.parse::<u32>().expect("valid full refresh cadence"));
    let mut selected = 0;
    for (case, width, height, scale, cover) in [
        ("text_1x", 1000, 600, 1.0, false),
        ("text_175", 1750, 1051, 1.75, false),
        ("cover_175", 1750, 1051, 1.75, true),
    ] {
        if only_case.as_deref().is_some_and(|selected| selected != case) { continue; }
        for full_every in [8, 3] {
            if only_every.is_some_and(|selected| selected != full_every) { continue; }
            for strips in STRIPS {
                if only_strips.is_some_and(|selected| selected != strips) { continue; }
                selected += 1;
                run_batch_case(
                    case, width, height, scale, cover, full_every, strips,
                );
            }
        }
    }
    assert!(selected > 0, "no matching A/B scene selected");
}


/// Product-relevance gate: never split damage into synthetic strips. Count how
/// often the normal Damage Tracker naturally produces full-edge neighbours
/// that the exact-coverage coalescer could join. A synthetic 16-strip win is
/// not evidence of any benefit to ordinary scrolling.
fn run_natural_case(case: &str, width: u32, height: u32, scale: f64, cover: bool, workers: u16) {
    let mut h = Harness::with(None, scale);
    h.root.handle_window_event(
        masonry::core::WindowEvent::Resize(PhysicalSize::new(width, height)),
    );
    h.app.window_resized(
        &mut h.root,
        Size::new(f64::from(width) / scale, f64::from(height) / scale),
    );
    let body: String = (0..250)
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
    let path = h.write_note("natural-adjacent.md", &format!("{frontmatter}{body}"));
    h.app.open(&mut h.root, path);
    if cover {
        h.wait_cover(true);
    }
    let _ = h.root.redraw();

    let (w, hh) = (width as u16, height as u16);
    let mut renderers = [Renderer::new(workers), Renderer::new(workers)];
    renderers[1].enable_adjacent_coalescing_for_test();
    let mut samples: [Vec<Timing>; 2] = std::array::from_fn(|_| Vec::new());
    let (mut active, mut scroll, mut multiple_regions, mut eligible, mut regions_saved) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut greatest_delta = 0u8;

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
        // The first frame seeds the two independent renderers. All later
        // damage comes from the unmodified production tracker and scroll path.
        let order = if step % 2 == 0 { [0, 1] } else { [1, 0] };
        let mut frame: [Option<Timing>; 2] = [None; 2];
        for index in order {
            frame[index] = Some(draw_batch(
                &mut renderers[index], &layers, w, hh, scale, step == 0,
            ));
        }
        let times = frame.map(|v| v.expect("both natural-damage renderers completed"));
        assert_eq!(
            times[0].pixels, times[1].pixels,
            "{case}/workers={workers}/step={step}: coalescing expanded raster area"
        );
        assert_eq!(
            renderers[0].moved, renderers[1].moved,
            "{case}/workers={workers}/step={step}: scroll movement diverged"
        );
        assert!(
            times[1].rects <= times[0].rects,
            "{case}/workers={workers}/step={step}: coalescing added rectangles"
        );
        let changed_partition = times[1].rects < times[0].rects;
        let peak = peak_channel(renderers[0].frame_pixels(), renderers[1].frame_pixels());
        greatest_delta = greatest_delta.max(peak);
        assert!(
            peak <= if changed_partition { 1 } else { 0 },
            "{case}/workers={workers}/step={step}: natural coalescing parity {peak}/255; \
             changed_partition={changed_partition}"
        );

        if changed_partition || step % 16 == 0 {
            let full = render_full(&layers, w, hh, scale);
            for (index, renderer) in renderers.iter().enumerate() {
                let delta = peak_channel(renderer.frame_pixels(), full.frame_pixels());
                assert!(
                    delta <= 1,
                    "{case}/workers={workers}/step={step}: mode {index} versus fresh full {delta}/255"
                );
            }
        }

        if step == 0 {
            continue;
        }
        if times[0].rects == 0 && renderers[0].moved.is_empty() {
            continue;
        }
        active += 1;
        scroll += usize::from(!renderers[0].moved.is_empty());
        multiple_regions += usize::from(times[0].rects >= 2);
        eligible += usize::from(changed_partition);
        regions_saved += times[0].rects - times[1].rects;
        for index in 0..2 {
            samples[index].push(times[index]);
        }
    }
    assert!(active >= 16, "{case}/workers={workers}: insufficient real frames");
    let [baseline_p50, baseline_p95] = quantiles(&samples[0], |t| t.total);
    let [merged_p50, merged_p95] = quantiles(&samples[1], |t| t.total);
    let baseline_sum: f64 = samples[0].iter().map(|t| t.total).sum();
    let merged_sum: f64 = samples[1].iter().map(|t| t.total).sum();
    eprintln!(
        "VELLO_NATURAL_ADJACENT case={case} workers={workers} frames={active} \
         scroll_frames={scroll} multiple_regions={multiple_regions} \
         eligible_frames={eligible} regions_saved={regions_saved} max_channel={greatest_delta} \
         baseline_p50_us={baseline_p50:.2} baseline_p95_us={baseline_p95:.2} \
         merged_p50_us={merged_p50:.2} merged_p95_us={merged_p95:.2} \
         baseline_sum_us={baseline_sum:.2} merged_sum_us={merged_sum:.2}"
    );
}

#[test]
#[ignore = "opt-in real-damage eligibility gate; no synthetic strips or production changes"]
fn benchmark_vello_natural_adjacent_regions() {
    for (case, w, h, scale, cover) in [
        ("text_1x", 1000, 600, 1.0, false),
        ("text_175", 1750, 1051, 1.75, false),
        ("cover_175", 1750, 1051, 1.75, true),
    ] {
        for workers in [0, 2] {
            run_natural_case(case, w, h, scale, cover, workers);
        }
    }
}
