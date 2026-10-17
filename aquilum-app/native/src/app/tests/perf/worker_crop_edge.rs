//! Test-only crop-boundary falsifier for the 8-strip header control defect.
//!
//! An archived release-mode failure on cover@1.75x / step 3 was entirely in
//! the first band at x=873..1122,y=77..130, touching the y=131 raster cut.
//! This test shifts just that cut while keeping exact screen coverage; it
//! measures which cutoff positions retain or eliminate the >1 LSB error.
//!
//! Deliberately diagnostic: any severe mismatch is logged and saved, not
//! silently reclassified as passing production frame fidelity.

use super::*;
use std::time::Duration;

const BIASES: [i32; 5] = [-48, -24, 0, 24, 48];
const W: u32 = 1750;
const H: u32 = 1051;
const SCALE: f64 = 1.75;
const STRIPS: u16 = 8;

fn screen_diff(
    left: &[u32],
    right: &[u32],
    width: usize,
) -> (usize, usize, u8, Option<(usize, usize, usize, usize)>) {
    assert_eq!(left.len(), right.len(), "invalid framebuffer dimensions");
    let (mut changed, mut severe, mut max) = (0usize, 0usize, 0u8);
    let mut box_severe: Option<(usize, usize, usize, usize)> = None;
    for (index, (&a, &b)) in left.iter().zip(right).enumerate() {
        if a == b { continue; }
        changed += 1;
        let diff = [0, 8, 16, 24]
            .into_iter()
            .map(|bits| ((a >> bits) as u8).abs_diff((b >> bits) as u8))
            .max()
            .unwrap_or(0);
        max = max.max(diff);
        if diff > 1 {
            severe += 1;
            let (x, y) = (index % width, index / width);
            box_severe = Some(match box_severe {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    (changed, severe, max, box_severe)
}

fn biased_header_case(bias: i32) {
    let mut h = Harness::with(None, SCALE);
    h.root.handle_window_event(
        masonry::core::WindowEvent::Resize(PhysicalSize::new(W, H)),
    );
    h.app.window_resized(
        &mut h.root,
        Size::new(f64::from(W) / SCALE, f64::from(H) / SCALE),
    );
    let text: String = (0..250)
        .map(|i| format!(
            "Абзац {i}: текст Akula, **жирный**, *курсив* и длинные Unicode \
             строки для переноса через Parley.\n\n"
        ))
        .collect();
    let path = h.write_note(
        "header-crop-probe.md",
        &format!(
            "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n{text}"
        ),
    );
    h.app.open(&mut h.root, path);
    h.wait_cover(true);
    let _ = h.root.redraw();

    let mut renderer = Renderer::new(0);
    renderer.split_full_frame_for_test(STRIPS);
    renderer.set_split_first_cut_bias_for_test(bias);

    for step in 0..=3_u32 {
        h.scroll(
            (f64::from(W) * 0.6, f64::from(H) * 0.5),
            SCALE,
            (0.0, -29.0),
        );
        h.anim(Duration::from_millis(16));
        let (layers, _) = h.root.redraw();
        let full = step == 0 || step == 3;
        renderer
            .draw_frame(&layers, W as u16, H as u16, SCALE, theme::background(), full)
            .expect("render 8-band probe frame");
        if step != 3 { continue; }
        assert_eq!(renderer.last_stages.raster_rects, usize::from(STRIPS));
        assert_eq!(
            renderer.last_stages.raster_pixels,
            u64::from(W) * u64::from(H),
            "biased strips must preserve the exact full-frame pixel coverage"
        );
        let reference = render_full(&layers, W as u16, H as u16, SCALE);
        let (changed, severe, peak, bbox) = screen_diff(
            renderer.frame_pixels(),
            reference.frame_pixels(),
            W as usize,
        );
        let base_cut = H / u32::from(STRIPS);
        let cut = i64::from(base_cut) + i64::from(bias);
        eprintln!(
            "VELLO_CROP_EDGE_BIAS bias={bias} first_cut_y={cut} \
             frame=3 strips={STRIPS} changed={changed} severe_gt1={severe} \
             max_channel={peak} severe_bbox={bbox:?}"
        );
        if severe > 0 {
            if let Some(path) = std::env::var_os("AQ_CROP_EDGE_ARTIFACTS") {
                let dir = std::path::PathBuf::from(path);
                std::fs::create_dir_all(&dir).expect("create artifact directory");
                std::fs::write(
                    dir.join(format!("bias{bias}-partial.png")),
                    renderer.png(),
                ).expect("partial PNG");
                std::fs::write(
                    dir.join(format!("bias{bias}-full.png")),
                    reference.png(),
                ).expect("reference PNG");
            }
        }
    }
}

#[test]
#[ignore = "diagnostic crop-boundary sweep, not the shipping renderer"]
fn diagnose_vello_cropped_header_layer() {
    for bias in BIASES {
        biased_header_case(bias);
    }
}
