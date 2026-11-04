//! Select a safe dispatcher for independently rendered, retained mask scenes.
//! The parent frame's damage preflight cannot see filters in a nested mask.

use masonry::imaging::record::{Command, Scene};
use vello_cpu::RenderSettings;

pub(super) fn settings(scene: &Scene, original: RenderSettings) -> RenderSettings {
    if original.num_threads == 0 || !contains_filters(scene) {
        return original;
    }
    RenderSettings { num_threads: 0, ..original }
}

// Scan once on each mask-cache miss. Walk nested retained masks too, without
// generating geometry/draw commands or paying for a second render.
fn contains_filters(scene: &Scene) -> bool {
    let mut pending = vec![scene];
    while let Some(current) = pending.pop() {
        for command in current.commands() {
            let Command::PushGroup(id) = command else { continue };
            let group = current.group(*id);
            if !group.filters.is_empty() {
                return true;
            }
            if let Some(applied) = &group.mask {
                pending.push(&current.mask(applied.mask).scene);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use masonry::imaging::{Filter, MaskMode};
    use masonry::imaging::record::{AppliedMask, Group, Mask as RecordedMask};
    use masonry::kurbo::{Affine, Rect};
    use masonry::peniko::Color;
    use vello_cpu::color::palette::css::RED;
    use vello_cpu::{Pixmap, RenderContext, Resources};

    fn scene_with_group(filters: Vec<Filter>) -> Scene {
        let mut scene = Scene::new();
        scene.push_group(Group { filters, ..Default::default() });
        masonry::imaging::Painter::new(&mut scene)
            .fill_rect(Rect::new(10.0, 10.0, 30.0, 30.0), Color::WHITE);
        scene.pop_group();
        scene
    }

    #[test]
    fn mask_filter_preflight_preserves_parallel_for_unfiltered_mask() {
        let scene = scene_with_group(vec![]);
        assert!(!contains_filters(&scene));
        assert_eq!(settings(&scene, crate::render::paint::settings(2)).num_threads, 2);
        assert_eq!(settings(&scene, crate::render::paint::settings(0)).num_threads, 0);
    }

    #[test]
    fn mask_filter_preflight_detects_filter_inside_nested_mask() {
        let inner = scene_with_group(vec![Filter::Offset { dx: 15.0, dy: 13.0 }]);
        let mut outer = scene_with_group(vec![]);
        let id = outer.define_mask(RecordedMask::new(MaskMode::Alpha, inner));
        outer.push_group(Group {
            mask: Some(AppliedMask::new(id)), ..Default::default()
        });
        masonry::imaging::Painter::new(&mut outer)
            .fill_rect(Rect::new(0.0, 0.0, 80.0, 80.0), Color::WHITE);
        outer.pop_group();
        assert!(contains_filters(&outer), "nested masked filters must be found");
        assert_eq!(settings(&outer, crate::render::paint::settings(2)).num_threads, 0);
    }

    fn render_mask(scene: &Scene, workers: u16) -> Pixmap {
        let mut cache = VecDeque::new();
        let mask = super::super::get_or_render_mask(
            &mut cache, scene, MaskMode::Alpha,
            Affine::IDENTITY, 80, 80, crate::render::paint::settings(workers),
        ).expect("filtered mask must render without dispatcher panic");
        assert_eq!(cache.len(), 1);
        let mut ctx = RenderContext::new_with(80, 80, crate::render::paint::settings(0));
        ctx.set_mask(mask);
        ctx.set_paint(RED);
        ctx.fill_rect(&Rect::new(0.0, 0.0, 80.0, 80.0));
        ctx.flush();
        let mut result = Pixmap::new(80, 80);
        ctx.render(&mut result, &mut Resources::new());
        result
    }

    #[test]
    fn mask_filter_preflight_rasterizes_two_offsets_with_multithreaded_parent() {
        let scene = scene_with_group(vec![
            Filter::Offset { dx: 15.0, dy: 0.0 },
            Filter::Offset { dx: 0.0, dy: 13.0 },
        ]);
        let reference = render_mask(&scene, 0);
        let actual = render_mask(&scene, 2);
        assert_eq!(actual.data_as_u8_slice(), reference.data_as_u8_slice());
        let point = actual.sample(30, 28);
        assert!(point.r > 200 && point.a > 200,
            "both offset filters must affect mask coverage: {point:?}");
        assert_eq!(actual.sample(15, 15).a, 0, "unfiltered original mask remains");
        assert_eq!(actual.sample(30, 15).a, 0, "only horizontal offset applied");
        assert_eq!(actual.sample(15, 28).a, 0, "only vertical offset applied");
    }
}
