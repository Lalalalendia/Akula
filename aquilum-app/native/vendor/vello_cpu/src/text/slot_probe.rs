// Copyright 2026
// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Diagnostic only. Never call without AQ_TEST_REAL_OUTLINE_SLOT_AB.
// Uses actual glifo::AtlasCommand::FillPath geometry captured from Akula's
// real editor, rather than the synthetic Bézier used by PR #205.
//
// This code does NOT write to the real atlas; it renders independent temporary
// pixmaps to test whether one exact outline yields the same normalized mask
// when its atlas slot or surrounding draw order changes.

use super::*;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use kurbo::Shape as _;

const PAGE_WIDTH: u16 = 160;
const PAGE_HEIGHT: u16 = 120;
// slot_3 proved one genuine 35-element outline is translation-sensitive.
// slot_4/5 vary X/Y independently; slot_6/7 probe integer/tile boundaries.
const TARGET_SLOTS: [(i32, i32); 8] = [
    (16, 16),
    (32, 24),
    (45, 37),
    (63, 51),
    (63, 16),
    (16, 51),
    (64, 51),
    (63, 52),
];
const NEIGHBOR_SLOT: (i32, i32) = (116, 78);
const WINDOW_START: i32 = -2;
const WINDOW_END: i32 = 30;
const MAX_REAL_OUTLINES_PER_PROCESS: usize = 80;

static PROBED: AtomicUsize = AtomicUsize::new(0);
static FIRST_REAL_FAILURE_LOGGED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct RealOutline<'a> {
    path: &'a BezPath,
    transform: Affine,
    bound_x: f64,
    bound_y: f64,
    command_index: usize,
}

fn paint_one(ctx: &mut RenderContext, outline: RealOutline<'_>, destination: (i32, i32)) {
    // Only the integer atlas slot changes: retain *exactly* the real glyph's
    // cubic contours, scale, reflection and subpixel phase.
    let delta = (
        f64::from(destination.0) - outline.bound_x,
        f64::from(destination.1) - outline.bound_y,
    );
    ctx.set_transform(Affine::translate(delta) * outline.transform);
    ctx.set_paint(BLACK);
    ctx.fill_path(outline.path);
}

fn render_one(
    level: Level,
    target: RealOutline<'_>,
    slot: (i32, i32),
    neighbor: Option<(RealOutline<'_>, bool)>,
) -> Pixmap {
    let mut ctx = RenderContext::new_with(
        PAGE_WIDTH,
        PAGE_HEIGHT,
        RenderSettings {
            level,
            num_threads: 0,
        },
    );
    if let Some((other, true)) = neighbor {
        paint_one(&mut ctx, other, NEIGHBOR_SLOT);
    }
    paint_one(&mut ctx, target, slot);
    if let Some((other, false)) = neighbor {
        paint_one(&mut ctx, other, NEIGHBOR_SLOT);
    }
    ctx.flush();
    let mut pixmap = Pixmap::new(PAGE_WIDTH, PAGE_HEIGHT);
    ctx.render(&mut pixmap, &mut Resources::new());
    pixmap
}

fn compare_masks(
    reference: &Pixmap,
    reference_slot: (i32, i32),
    candidate: &Pixmap,
    candidate_slot: (i32, i32),
) -> (usize, u8, Option<(i32, i32, u32, u32)>) {
    let mut changed = 0;
    let mut maximum = 0_u8;
    let mut first = None;
    for dy in WINDOW_START..WINDOW_END {
        for dx in WINDOW_START..WINDOW_END {
            let a = reference
                .sample((reference_slot.0 + dx) as u16, (reference_slot.1 + dy) as u16)
                .to_u32();
            let b = candidate
                .sample((candidate_slot.0 + dx) as u16, (candidate_slot.1 + dy) as u16)
                .to_u32();
            if a != b {
                changed += 1;
                if first.is_none() {
                    first = Some((dx, dy, a, b));
                }
                for (aa, bb) in a.to_le_bytes().iter().zip(b.to_le_bytes()) {
                    maximum = maximum.max(aa.abs_diff(bb));
                }
            }
        }
    }
    (changed, maximum, first)
}

/// Inspect the real outlines of pending cache misses *before* normal atlas
/// preparation changes the recorded transform by its dirty-bounds shift.
///
/// This is a falsifier, not a test pass condition: any changed mask is reported,
/// and the real strict guard96 parity test remains independent.
pub(super) fn run(commands: &[AtlasCommand], level: Level, page_index: u32) {
    if std::env::var_os("AQ_TEST_REAL_OUTLINE_SLOT_AB").is_none() {
        return;
    }

    let mut current = Affine::IDENTITY;
    let mut real = Vec::new();
    for (command_index, command) in commands.iter().enumerate() {
        match command {
            AtlasCommand::SetTransform(t) => current = *t,
            AtlasCommand::FillPath(path) => {
                let bounds = current.transform_rect_bbox(path.bounding_box());
                let (width, height) = (bounds.width(), bounds.height());
                if !bounds.x0.is_finite()
                    || !bounds.y0.is_finite()
                    || !(4.0..=22.0).contains(&width)
                    || !(5.0..=24.0).contains(&height)
                {
                    continue;
                }
                real.push(RealOutline {
                    path: path.as_ref(),
                    transform: current,
                    bound_x: bounds.x0.floor(),
                    bound_y: bounds.y0.floor(),
                    command_index,
                });
            }
            _ => {}
        }
    }

    for (index, &outline) in real.iter().enumerate() {
        let sample = PROBED.fetch_add(1, Ordering::Relaxed);
        if sample >= MAX_REAL_OUTLINES_PER_PROCESS {
            break;
        }
        let neighbor = real[(index + 1) % real.len()];
        let reference = render_one(level, outline, TARGET_SLOTS[0], None);

        for (variant, &slot) in TARGET_SLOTS.iter().enumerate().skip(1) {
            let candidate = render_one(level, outline, slot, None);
            let (changed, peak, first) = compare_masks(
                &reference,
                TARGET_SLOTS[0],
                &candidate,
                slot,
            );
            if changed > 0 && !FIRST_REAL_FAILURE_LOGGED.swap(true, Ordering::Relaxed) {
                // The first true source-font outline that fails exact slot
                // invariance. Emit its full path once for a reusable fixture.
                // Do not log font names or user document content.
                std::eprintln!(
                    "VELLO_REAL_OUTLINE_FIXTURE sample={sample} variant=slot_{variant} \
                     src_transform={:?} src_bounds=({:.12},{:.12}) \
                     page={page_index} cmd={} path_elements={:?}",
                    outline.transform.as_coeffs(),
                    outline.bound_x,
                    outline.bound_y,
                    outline.command_index,
                    outline.path.elements(),
                );
            }
            std::eprintln!(
                "VELLO_REAL_OUTLINE sample={sample} page={page_index} cmd={} \
                 elements={} variant=slot_{variant} slot={slot:?} \
                 changed={changed} peak={peak} first={first:?}",
                outline.command_index,
                outline.path.elements().len(),
            );
        }
        // The other shape is placed far from the target's pixel window.
        // This catches ordering / batching state effects without shape overlap.
        for (name, other_first) in [("before", true), ("after", false)] {
            let candidate = render_one(
                level,
                outline,
                TARGET_SLOTS[0],
                Some((neighbor, other_first)),
            );
            let (changed, peak, first) = compare_masks(
                &reference,
                TARGET_SLOTS[0],
                &candidate,
                TARGET_SLOTS[0],
            );
            std::eprintln!(
                "VELLO_REAL_OUTLINE sample={sample} page={page_index} cmd={} \
                 elements={} variant=batch_{name} neighbor_cmd={} \
                 changed={changed} peak={peak} first={first:?}",
                outline.command_index,
                outline.path.elements().len(),
                neighbor.command_index,
            );
        }
    }
}
