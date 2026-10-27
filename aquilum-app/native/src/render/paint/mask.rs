//! Кэширование и отрисовка масок групп.
use std::collections::VecDeque;

use masonry::imaging::MaskMode;
use masonry::imaging::record::{Scene, replay_transformed};
use masonry::kurbo::Affine;
use vello_cpu::{Mask, Pixmap, RenderSettings};

use super::{Painter, error::PaintError};
mod cache_budget;
mod filter_preflight;

#[derive(Clone)]
pub struct CachedMask {
    pub scene: Scene,
    pub mode: MaskMode,
    pub transform: Affine,
    pub mask: Mask,
}

pub const MASK_CACHE_LIMIT: usize = 8;
#[cfg(test)]
mod failure_propagation_tests;
pub fn get_or_render_mask(
    cache: &mut VecDeque<CachedMask>,
    scene: &Scene,
    mode: MaskMode,
    transform: Affine,
    width: u16,
    height: u16,
    settings: RenderSettings,
) -> Result<Mask, PaintError> {
    if let Some(entry) = cache.iter().find(|e| e.mode == mode && e.transform == transform && e.scene == *scene) {
        return Ok(entry.mask.clone());
    }

    let keep_cached = cache_budget::prepare(cache, width, height, MASK_CACHE_LIMIT);
    let mut sub = Painter::new(width, height, filter_preflight::settings(scene, settings));
    replay_transformed(scene, &mut sub, transform);
    sub.finish()?;
    let mut pixmap = Pixmap::new(width, height);
    sub.ctx.render(&mut pixmap, &mut sub.resources);
    let mask = match mode {
        MaskMode::Alpha => Mask::new_alpha(&pixmap),
        MaskMode::Luminance => Mask::new_luminance(&pixmap),
    };
    if keep_cached { cache.push_back(CachedMask { scene: scene.clone(), mode, transform, mask: mask.clone() }); }
    Ok(mask)
}

#[cfg(test)]
mod filter_mask_origin_tests {
    use std::sync::Arc;

    use vello_cpu::color::palette::css::{RED, WHITE};
    use vello_cpu::filter_effects::{Filter, FilterGraph, FilterPrimitive};
    use vello_cpu::kurbo::Rect;
    use vello_cpu::{Mask, Pixmap, RenderContext, Resources};

    const WIDTH: u16 = 128;
    const HEIGHT: u16 = 128;

    fn mask() -> Mask {
        let mut ctx = RenderContext::new_with(WIDTH, HEIGHT, super::super::settings(0));
        ctx.set_paint(WHITE);
        ctx.fill_rect(&Rect::new(40.0, 40.0, 100.0, 100.0));
        ctx.flush();
        let mut pix = Pixmap::new(WIDTH, HEIGHT);
        ctx.render(&mut pix, &mut Resources::new());
        Mask::new_alpha(&pix)
    }

    fn offset_filter() -> Filter {
        let mut graph = FilterGraph::new();
        let output = graph.add(FilterPrimitive::Offset { dx: -20.0, dy: -30.0 }, None);
        graph.set_output(output);
        Filter { graph: Arc::new(graph) }
    }

    fn frame(filtered: bool, direct_mask: bool) -> Vec<u8> {
        // The mask covers the original 40..100 rectangle in *root* coordinates;
        // the filter shifts the masked result to 20..80, 10..70.
        let mut ctx = RenderContext::new_with(WIDTH, HEIGHT, super::super::settings(0));
        if filtered {
            ctx.push_filter_layer(offset_filter());
        }
        if direct_mask {
            ctx.set_mask(mask());
        } else {
            ctx.push_mask_layer(mask());
        }
        ctx.set_paint(RED);
        ctx.fill_rect(&Rect::new(40.0, 40.0, 100.0, 100.0));
        if direct_mask {
            ctx.reset_mask();
        } else {
            ctx.pop_layer();
        }
        if filtered {
            ctx.pop_layer();
        }
        ctx.flush();
        let mut pix = Pixmap::new(WIDTH, HEIGHT);
        ctx.render(&mut pix, &mut Resources::new());
        pix.data_as_u8_slice().to_vec()
    }

    fn check_shifted_pixels(before: &[u8], shifted: &[u8], case: &str) {
        let width = usize::from(WIDTH);
        assert_eq!(before.len(), width * usize::from(HEIGHT) * 4);
        assert_eq!(shifted.len(), before.len());
        let mut different = 0;
        let mut worst = 0u8;
        for y in 0..usize::from(HEIGHT) {
            for x in 0..width {
                let source_x = x + 20;
                let source_y = y + 30;
                let expected: &[u8] = if source_x < width && source_y < usize::from(HEIGHT) {
                    let offset = (source_y * width + source_x) * 4;
                    &before[offset..offset + 4]
                } else {
                    &[0, 0, 0, 0]
                };
                let offset = (y * width + x) * 4;
                let delta = shifted[offset..offset + 4]
                    .iter()
                    .zip(expected)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                if delta > 2 {
                    different += 1;
                    if different <= 4 {
                        eprintln!("Vello #2004 {case}: pixel ({x},{y}) differs by {delta}/255");
                    }
                }
                worst = worst.max(delta);
            }
        }
        // An all-transparent "success" would hide the original disappearing-mask failure.
        let center = ((40 * width + 50) * 4) as usize;
        assert!(shifted[center] > 200 && shifted[center + 3] > 200,
            "{case}: masked red center vanished after the offset filter");
        assert_eq!(different, 0, "Vello #2004 {case}: {different} shifted pixels differ >2/255 (worst {worst})");
    }

    #[test]
    fn nested_mask_layer_in_offset_filter_uses_root_coordinates() {
        check_shifted_pixels(&frame(false, false), &frame(true, false), "mask layer");
    }

    #[test]
    fn set_mask_inside_offset_filter_uses_root_coordinates() {
        check_shifted_pixels(&frame(false, true), &frame(true, true), "set_mask");
    }
}
