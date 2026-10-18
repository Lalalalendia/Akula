use masonry::core::BrushIndex;
use masonry::kurbo::Size;
use masonry::parley::{Alignment, AlignmentOptions, FontContext, Layout, LayoutContext};

use super::text::{BOLD, TextStyle, glyph_width, wrapped};
use super::{Item, Line, Settings};
use crate::reader::flow::{Align, Block, Role};

const THREADS: usize = 4;

fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 2.0,
        2 => 1.5,
        3 => 1.17,
        5 => 0.83,
        6 => 0.67,
        _ => 1.0,
    }
}

fn alignment(align: Option<Align>, role: Role, justify: bool) -> Alignment {
    match (align, role) {
        (Some(Align::Center), _) | (None, Role::Heading(_)) => Alignment::Center,
        (Some(Align::End), _) => Alignment::End,
        (Some(Align::Start), _) => Alignment::Start,
        (None, Role::Pre) => Alignment::Start,
        _ if justify => Alignment::Justify,
        _ => Alignment::Start,
    }
}

fn spacing(role: Role, size: f32, em: f64) -> (f64, f64) {
    match role {
        Role::Heading(_) => (f64::from(size) * 0.67, f64::from(size) * 0.67),
        Role::Pre | Role::Item | Role::Cell => (em * 0.25, em * 0.25),
        Role::Paragraph => (0.0, 0.0),
    }
}

fn font_size(role: Role, settings: &Settings) -> f32 {
    match role {
        Role::Heading(level) => settings.size * heading_scale(level),
        _ => settings.size,
    }
}

fn image_item(natural: Option<(u32, u32)>, width: f64, height: f64, em: f64) -> Item {
    let size = natural.map_or(Size::ZERO, |(w, h)| {
        let (w, h) = (f64::from(w), f64::from(h));
        let k = (width / w).min(height / h).min(1.0);
        Size::new(w * k, h * k)
    });
    Item::Image { size, before: em * 0.5, after: em * 0.5 }
}

pub fn dash_width(fcx: &mut FontContext, lcx: &mut LayoutContext<BrushIndex>, settings: &Settings) -> f32 {
    glyph_width(fcx, lcx, "-", crate::ui::theme::editor_font(&settings.family), settings.size, settings.weight)
}

pub fn text_layout(block: &Block, fcx: &mut FontContext, lcx: &mut LayoutContext<BrushIndex>, settings: &Settings, width: f64, dash: f32) -> Option<(Layout<BrushIndex>, Vec<(usize, char)>, f64)> {
    let Block::Text { text, spans, role, depth, indent, align, .. } = block else { return None };
    let mono = crate::ui::theme::editor_font("iA Writer Mono");
    let x = f64::from(*depth) * f64::from(settings.size) * 1.5;
    let wrap = (width - x).max(1.0) as f32;
    let style = TextStyle {
        font: if *role == Role::Pre { mono.clone() } else { crate::ui::theme::editor_font(&settings.family) },
        mono,
        size: font_size(*role, settings),
        weight: if matches!(role, Role::Heading(_)) { BOLD } else { settings.weight },
        line_height: settings.line_height,
        indent: indent.then_some(settings.size),
        dash,
    };
    let hyphenated = settings.hyphenate && matches!(role, Role::Paragraph | Role::Item | Role::Cell);
    let (mut layout, breaks) = wrapped(fcx, lcx, text, spans, hyphenated, &style, wrap);
    layout.align(alignment(*align, *role, settings.justify), AlignmentOptions::default());
    Some((layout, breaks, x))
}

// The reader previously split layout work into equal block-count chunks.
// EPUB generators can put most of a book in a handful of giant <br>-joined
// blocks; one Rayon worker then determines the total latency.
fn block_weight(block: &Block) -> usize {
    match block {
        Block::Text { text, .. } => text.len().max(1),
        Block::Image { .. } | Block::Rule => 1,
    }
}

fn fixed_ranges(len: usize) -> Vec<std::ops::Range<usize>> {
    let chunk = len.div_ceil(THREADS).max(1);
    (0..len).step_by(chunk).map(|start| start..(start + chunk).min(len)).collect()
}

// Partition in source order, minimizing the largest approximate workload.
// Preserving order is required for CFI positions and downstream pagination.
fn weighted_ranges(weights: &[usize], workers: usize) -> Vec<std::ops::Range<usize>> {
    if weights.is_empty() {
        return Vec::new();
    }
    let groups = workers.min(weights.len()).max(1);
    let mut low = *weights.iter().max().expect("nonempty weights");
    let mut high = weights.iter().fold(0usize, |sum, &weight| sum.saturating_add(weight));
    while low < high {
        let limit = low + (high - low) / 2;
        let (mut groups_used, mut used) = (1usize, 0usize);
        for &weight in weights {
            if used > limit - weight {
                groups_used += 1;
                used = weight;
            } else {
                used += weight;
            }
        }
        if groups_used <= groups {
            high = limit;
        } else {
            low = limit + 1;
        }
    }
    let mut result = Vec::with_capacity(groups);
    let (mut start, mut used, mut remaining_groups) = (0usize, 0usize, groups);
    for (index, &weight) in weights.iter().enumerate() {
        if remaining_groups > 1
            && index > start
            && (used > low - weight || weights.len() - index == remaining_groups - 1)
        {
            result.push(start..index);
            start = index;
            used = 0;
            remaining_groups -= 1;
        }
        used += weight;
    }
    result.push(start..weights.len());
    debug_assert_eq!(result.len(), groups);
    result
}

fn layout_ranges(blocks: &[Block], weighted: bool) -> Vec<std::ops::Range<usize>> {
    let fixed = fixed_ranges(blocks.len());
    // Keep the fast and familiar scheduling path for typical small chapters.
    if !weighted || blocks.len() <= THREADS {
        return fixed;
    }
    let weights: Vec<usize> = blocks.iter().map(block_weight).collect();
    if weights.iter().copied().max().unwrap_or(0) < 16 * 1024 {
        return fixed;
    }
    let balanced = weighted_ranges(&weights, THREADS);
    let load = |range: &std::ops::Range<usize>| {
        weights[range.clone()].iter().fold(0usize, |sum, &w| sum.saturating_add(w))
    };
    let previous_peak = fixed.iter().map(&load).max().unwrap_or(0);
    let balanced_peak = balanced.iter().map(&load).max().unwrap_or(0);
    // Weight is only a proxy for shaping/hyphenation cost. Avoid churn unless
    // the predicted worst worker has materially less input to process.
    if balanced_peak < previous_peak.saturating_sub(previous_peak / 8) {
        balanced
    } else {
        fixed
    }
}

// Retain only first-page candidates. The source-byte budget limits
// speculative retention; it is not an exact allocator/RSS measurement.
// We use this path only for a giant paragraph near the beginning.
pub(super) const REUSE_LOOKAHEAD_BLOCKS: usize = 24;
pub(super) const REUSE_SOURCE_BUDGET: usize = 256 * 1024;
pub(super) const REUSE_BLOCK_LIMIT: usize = 128 * 1024;
const REUSE_MIN_GIANT: usize = 16 * 1024;

fn initial_candidates(blocks: &[Block]) -> Vec<bool> {
    let mut flags = vec![false; blocks.len()];
    if !blocks.iter().take(REUSE_LOOKAHEAD_BLOCKS).any(
        |block| matches!(block, Block::Text { text, .. } if text.len() >= REUSE_MIN_GIANT)
    ) {
        return flags;
    }
    let mut used = 0usize;
    for (index, block) in blocks.iter().take(REUSE_LOOKAHEAD_BLOCKS).enumerate() {
        if let Block::Text { text, .. } = block {
            let bytes = text.len();
            if bytes <= REUSE_BLOCK_LIMIT && bytes <= REUSE_SOURCE_BUDGET - used {
                flags[index] = true;
                used += bytes;
            }
        }
    }
    flags
}

fn layout_blocks_impl(
    blocks: &[Block],
    images: &mut dyn FnMut(&str) -> Option<(u32, u32)>,
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<BrushIndex>,
    settings: &Settings,
    width: f64,
    height: f64,
    weighted: bool,
) -> Vec<Item> {
    use rayon::prelude::*;
    let (em, dash) = (f64::from(settings.size), dash_width(fcx, lcx, settings));
    let item = |block: &Block, natural: Option<(u32, u32)>, fcx: &mut FontContext, lcx: &mut LayoutContext<BrushIndex>| match (block, text_layout(block, fcx, lcx, settings, width, dash)) {
        (Block::Text { role, .. }, Some((layout, breaks, x))) => {
            let lines = layout
                .lines()
                .map(|line| {
                    let (metrics, range) = (line.metrics(), line.text_range());
                    Line { top: f64::from(metrics.block_min_coord), bottom: f64::from(metrics.block_max_coord), start: range.start, end: range.end }
                })
                .collect();
            let (before, after) = spacing(*role, font_size(*role, settings), em);
            Item::Text { lines, x, before, after, breaks }
        }
        (Block::Image { .. }, _) => image_item(natural, width, height, em),
        _ => Item::Rule,
    };
    let sizes: Vec<Option<(u32, u32)>> = blocks.iter().map(|block| if let Block::Image { src, .. } = block { images(src) } else { None }).collect();
    let ranges = layout_ranges(blocks, weighted);
    let contexts: Vec<FontContext> = ranges.iter().map(|_| fcx.clone()).collect();
    let parts: Vec<Vec<Item>> = ranges
        .into_par_iter()
        .zip(contexts)
        .map(|(range, mut fcx)| {
            let mut lcx = LayoutContext::new();
            blocks[range.clone()].iter().zip(&sizes[range]).map(|(block, &natural)| item(block, natural, &mut fcx, &mut lcx)).collect()
        })
        .collect();
    parts.into_iter().flatten().collect()
}



// Capture the exact Parley layouts produced for a bounded set of prospective
// first-page blocks while computing page geometry. Later Session::show can
// adopt these if they actually land on-screen; all others are dropped.
pub fn layout_blocks_with_candidates(
    blocks: &[Block],
    images: &mut dyn FnMut(&str) -> Option<(u32, u32)>,
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<BrushIndex>,
    settings: &Settings,
    width: f64,
    height: f64,
) -> (Vec<Item>, Vec<(usize, Layout<BrushIndex>)>) {
    use rayon::prelude::*;
    let candidates = initial_candidates(blocks);
    if !candidates.contains(&true) {
        return (layout_blocks(blocks, images, fcx, lcx, settings, width, height), Vec::new());
    }
    let (em, dash) = (f64::from(settings.size), dash_width(fcx, lcx, settings));
    let sizes: Vec<Option<(u32, u32)>> = blocks.iter().map(
        |block| if let Block::Image { src, .. } = block { images(src) } else { None }
    ).collect();
    let ranges = layout_ranges(blocks, true);
    let contexts: Vec<FontContext> = ranges.iter().map(|_| fcx.clone()).collect();
    let parts: Vec<Vec<(Item, Option<Layout<BrushIndex>>)>> = ranges
        .into_par_iter()
        .zip(contexts)
        .map(|(range, mut fcx)| {
            let mut lcx = LayoutContext::new();
            blocks[range.clone()]
                .iter()
                .zip(&sizes[range.clone()])
                .enumerate()
                .map(|(offset, (block, &natural))| {
                    let index = range.start + offset;
                    match (block, text_layout(block, &mut fcx, &mut lcx, settings, width, dash)) {
                        (Block::Text { role, .. }, Some((layout, breaks, x))) => {
                            let lines = layout
                                .lines()
                                .map(|line| {
                                    let (metrics, range) = (line.metrics(), line.text_range());
                                    Line {
                                        top: f64::from(metrics.block_min_coord),
                                        bottom: f64::from(metrics.block_max_coord),
                                        start: range.start,
                                        end: range.end,
                                    }
                                })
                                .collect();
                            let (before, after) = spacing(*role, font_size(*role, settings), em);
                            let retained = candidates[index].then_some(layout);
                            (Item::Text { lines, x, before, after, breaks }, retained)
                        }
                        (Block::Image { .. }, _) => (image_item(natural, width, height, em), None),
                        _ => (Item::Rule, None),
                    }
                })
                .collect()
        })
        .collect();
    let mut items = Vec::with_capacity(blocks.len());
    let mut retained = Vec::new();
    for (index, (item, layout)) in parts.into_iter().flatten().enumerate() {
        items.push(item);
        if let Some(layout) = layout {
            retained.push((index, layout));
        }
    }
    (items, retained)
}


pub fn layout_blocks(
    blocks: &[Block],
    images: &mut dyn FnMut(&str) -> Option<(u32, u32)>,
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<BrushIndex>,
    settings: &Settings,
    width: f64,
    height: f64,
) -> Vec<Item> {
    layout_blocks_impl(blocks, images, fcx, lcx, settings, width, height, true)
}

// Baseline is compiled only for in-process A/B and byte-for-byte geometry QA.
#[cfg(test)]
pub(super) fn layout_blocks_fixed(
    blocks: &[Block],
    images: &mut dyn FnMut(&str) -> Option<(u32, u32)>,
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<BrushIndex>,
    settings: &Settings,
    width: f64,
    height: f64,
) -> Vec<Item> {
    layout_blocks_impl(blocks, images, fcx, lcx, settings, width, height, false)
}

#[cfg(test)]
mod partition_tests {
    use super::weighted_ranges;

    #[test]
    fn weighted_ranges_cover_items_once_in_original_order() {
        // Exhaustive small weights, nonempty partitions, 1–4 workers.
        for len in 1usize..=7 {
            let combinations = 4usize.pow(len as u32);
            for value in 0..combinations {
                let mut value = value;
                let weights: Vec<usize> = (0..len).map(|_| {
                    let weight = [1, 2, 5, 20][value % 4];
                    value /= 4;
                    weight
                }).collect();
                for workers in 1..=4 {
                    let ranges = weighted_ranges(&weights, workers);
                    assert_eq!(ranges.len(), workers.min(len));
                    let expanded: Vec<usize> = ranges.iter().flat_map(|r| r.clone()).collect();
                    assert_eq!(expanded, (0..len).collect::<Vec<_>>());
                    assert!(ranges.iter().all(|r| !r.is_empty()));
                    let peak = ranges.iter().map(|r| weights[r.clone()].iter().sum::<usize>()).max().unwrap();
                    assert!(peak <= weights.iter().sum::<usize>());
                }
            }
        }
    }

    #[test]
    fn heavy_tail_balances_better_than_equal_block_counts() {
        let weights = [118_000, 110_000, 101_000, 97_000, 90_000, 85_000, 80_000, 74_000, 62_000, 58_000, 1, 1, 1, 1, 1, 1];
        let weighted = weighted_ranges(&weights, 4);
        let fixed_chunk = weights.len().div_ceil(4);
        let fixed_peak = weights.chunks(fixed_chunk).map(|c| c.iter().sum::<usize>()).max().unwrap();
        let weighted_peak = weighted.iter().map(|range| weights[range.clone()].iter().sum::<usize>()).max().unwrap();
        assert!(weighted_peak < fixed_peak);
    }
}


#[cfg(test)]
mod candidate_tests {
    use super::{initial_candidates, REUSE_LOOKAHEAD_BLOCKS, REUSE_SOURCE_BUDGET, REUSE_BLOCK_LIMIT};
    use crate::reader::flow::{Block, Role};

    fn block(size: usize) -> Block {
        Block::Text {
            text: "X".repeat(size),
            spans: Vec::new(),
            origins: Vec::new(),
            role: Role::Paragraph,
            depth: 0,
            indent: false,
            align: None,
        }
    }

    #[test]
    fn no_capture_for_ordinary_books_or_late_giant_blocks() {
        let mut small: Vec<Block> = (0..25).map(|_| block(100)).collect();
        assert!(initial_candidates(&small).iter().all(|&b| !b));
        small[REUSE_LOOKAHEAD_BLOCKS] = block(40 * 1024);
        assert!(initial_candidates(&small).iter().all(|&b| !b));
    }

    #[test]
    fn speculative_retention_is_limited_by_source_bytes_and_indices() {
        let mut blocks: Vec<Block> = (0..32).map(|_| block(4096)).collect();
        blocks[1] = block(118 * 1024);
        blocks[2] = block(118 * 1024);
        blocks[3] = block(REUSE_BLOCK_LIMIT + 1);
        let flags = initial_candidates(&blocks);
        assert!(flags[0] && flags[1] && flags[2]);
        assert!(!flags[3]);
        assert!(flags[REUSE_LOOKAHEAD_BLOCKS..].iter().all(|&flag| !flag));
        let budget: usize = blocks.iter().zip(flags).filter_map(
            |(block, keep)| keep.then_some(super::block_weight(block))
        ).sum();
        assert!(budget <= REUSE_SOURCE_BUDGET);
    }
}
