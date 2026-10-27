//! Track aggregate live filter-layer working space across nested groups.
use super::PaintError;

const MAX_GROUP_DEPTH: usize = 4096;

#[derive(Default, Debug)]
pub(super) struct FilterBudgetStack {
    pixels: f64,
    layers: Vec<f64>,
}

impl FilterBudgetStack {
    pub(super) fn admit(&mut self, pixels: f64, limit: f64) -> Result<(), PaintError> {
        let new_total = self.pixels + pixels;
        if !pixels.is_finite() || pixels < 0.0 || !new_total.is_finite()
            || new_total > limit || self.layers.len() >= MAX_GROUP_DEPTH {
            return Err(PaintError::FilterBudget("совокупный размер вложенных фильтров превышает бюджет"));
        }
        self.layers.push(pixels);
        self.pixels = new_total;
        Ok(())
    }
    pub(super) fn exit(&mut self) {
        if let Some(last) = self.layers.pop() {
            self.pixels = (self.pixels - last).max(0.0);
        }
    }
    pub(super) fn clear(&mut self) {
        self.pixels = 0.0;
        self.layers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::paint::{Painter, settings};
    use masonry::imaging::Filter;
    use masonry::imaging::record::{replay_transformed, Group, Scene};
    use masonry::kurbo::{Affine, Rect};
    use masonry::peniko::Color;

    const CAP: f64 = 96.0 * 1024.0 * 1024.0;
    fn blur(s: f32) -> Filter {
        Filter::Blur { std_deviation_x: s, std_deviation_y: s }
    }

    #[test]
    fn nested_filter_work_budget_rejects_cumulative_individually_valid_layers() {
        let amount = super::super::validate(&[blur(850.0)], Affine::IDENTITY, 128, 128).unwrap();
        assert!(amount > 25_000_000.0 && amount < CAP / 2.0);
        let mut stack = FilterBudgetStack::default();
        for _ in 0..3 { stack.admit(amount, CAP).unwrap(); }
        assert!(matches!(stack.admit(amount, CAP), Err(PaintError::FilterBudget(_))));
        assert_eq!(stack.layers.len(), 3);
        stack.exit();
        stack.admit(amount, CAP).unwrap();
    }

    #[test]
    fn nested_filter_work_budget_enforces_depth_and_resets_after_region() {
        let mut stack = FilterBudgetStack::default();
        for _ in 0..MAX_GROUP_DEPTH { stack.admit(0.0, CAP).unwrap(); }
        assert!(stack.admit(0.0, CAP).is_err());
        stack.clear();
        assert_eq!((stack.pixels, stack.layers.len()), (0.0, 0));
        let mut painter = Painter::new(8, 8, settings(0));
        painter.filter_budget_stack.admit(CAP - 1.0, CAP).unwrap();
        painter.begin(0, 0, 8, 8);
        assert_eq!(painter.filter_budget_stack.pixels, 0.0);
        painter.filter_budget_stack.admit(1.0, CAP).unwrap();
        painter.trim();
        assert_eq!(painter.filter_budget_stack.pixels, 0.0);
    }

    #[test]
    fn nested_filter_work_budget_fails_real_nested_scene_before_vello_flush() {
        let mut scene = Scene::new();
        for _ in 0..4 {
            scene.push_group(Group { filters: vec![blur(850.0)], ..Default::default() });
        }
        masonry::imaging::Painter::new(&mut scene).fill_rect(
            Rect::new(10.0, 10.0, 50.0, 50.0), Color::WHITE,
        );
        for _ in 0..4 { scene.pop_group(); }
        let mut painter = Painter::new(128, 128, settings(0));
        painter.begin(0, 0, 128, 128);
        replay_transformed(&scene, &mut painter, Affine::IDENTITY);
        assert!(matches!(painter.finish(), Err(PaintError::FilterBudget(_))));
    }

    #[test]
    fn nested_filter_work_budget_retains_8k_and_unfiltered_paths() {
        let cost = super::super::validate(&[blur(3.0)], Affine::IDENTITY, 7680, 4320).unwrap();
        assert!(cost > 0.0 && cost < CAP);
        let mut stack = FilterBudgetStack::default();
        stack.admit(0.0, CAP).unwrap();
        stack.admit(cost, CAP).unwrap();
        stack.exit();
        stack.exit();
        assert_eq!(stack.pixels, 0.0);
        assert_eq!(super::super::validate(&[], Affine::IDENTITY, 3840, 2160).unwrap(), 0.0);
    }
}
