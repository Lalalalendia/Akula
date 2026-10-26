//! Masonry multi-filter groups on Vello CPU's single-primitive filter API.
//! Vello's PreparedFilter explicitly panics if a graph has >1 primitive.
//! Compose groups via nested filter layers, from outermost last filter to
//! innermost first filter, so the original CSS-like filter order is retained.

use masonry::imaging::Filter;
use masonry::kurbo::BezPath;
use masonry::peniko::BlendMode;
use vello_cpu::Mask;

use super::Painter;
use super::convert;
use super::error::PaintError;

impl Painter {
    /// Keep the existing one-filter path bit-for-bit unchanged. With multiple
    /// filters, apply blend/opacity/clip/mask only to the outer composite layer,
    /// then apply filter[0], filter[1], ... in the intended source order.
    pub(super) fn push_filter_chain(
        &mut self,
        clip: Option<&BezPath>,
        blend: Option<BlendMode>,
        opacity: Option<f32>,
        mask: Option<Mask>,
        filters: &[Filter],
    ) {
        if filters.len() <= 1 {
            self.ctx.push_layer(clip, blend, opacity, mask, convert::filters_to_vello(filters));
        } else {
            self.ctx.push_layer(clip, blend, opacity, mask, None);
            for filter in filters.iter().rev() {
                let single = convert::filters_to_vello(core::slice::from_ref(filter))
                    .expect("one Masonry filter maps to one Vello primitive");
                self.ctx.push_filter_layer(single);
            }
        }
        self.groups.push(filters.len());
    }

    /// Balance the encoded stack even for nested filtered groups. Error
    /// detection is preserved; never pop a Vello layer if no group was opened.
    pub(super) fn pop_filter_chain(&mut self) -> Result<(), PaintError> {
        let Some(n) = self.groups.pop() else {
            return Err(PaintError::Unbalanced("pop_group без push_group"));
        };
        if n > 1 {
            for _ in 0..n {
                self.ctx.pop_layer();
            }
        }
        self.ctx.pop_layer();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::kurbo::Rect;
    use vello_cpu::color::palette::css::RED;
    use vello_cpu::{Pixmap, RenderSettings};

    fn frame(workers: u16, filters: &[Filter]) -> Pixmap {
        let mut painter = Painter::new(1, 1, super::super::settings(workers));
        // Damage preflight must already have admitted filters to the serial
        // dispatcher before group commands are encoded.
        painter.route_for_scene(&[(0, 0, 80, 80)], !filters.is_empty());
        assert_eq!(
            painter.ctx.render_settings().num_threads,
            if filters.is_empty() { workers } else { 0 },
        );
        painter.begin(0, 0, 80, 80);
        painter.push_filter_chain(None, Some(BlendMode::default()), Some(1.0), None, filters);
        assert_eq!(painter.groups, vec![filters.len()]);
        painter.ctx.set_paint(RED);
        painter.ctx.fill_rect(&Rect::new(10.0, 10.0, 20.0, 20.0));
        painter.pop_filter_chain().expect("balanced filter group");
        assert!(painter.groups.is_empty());
        painter.finish().expect("flush filter layers");
        let mut output = Pixmap::new(80, 80);
        painter.ctx.render(&mut output, &mut painter.resources);
        output
    }

    #[test]
    fn multi_filter_chain_offsets_compose_without_vello_graph_panic() {
        let offsets = [
            Filter::Offset { dx: 15.0, dy: 0.0 },
            Filter::Offset { dx: 0.0, dy: 13.0 },
        ];
        // Prior implementation formed a two-primitive FilterGraph that
        // unconditionally panicked inside PreparedFilter::new.
        let serial = frame(0, &offsets);
        let routed = frame(2, &offsets);
        assert_eq!(serial.data_as_u8_slice(), routed.data_as_u8_slice());
        let center = routed.sample(30, 28);
        assert!(center.r > 200 && center.a > 200,
            "both offsets must move the source to (25..35,23..33), got {center:?}");
        assert_eq!(routed.sample(15, 15).a, 0,
            "source position must be cleared after both offsets");
        assert_eq!(routed.sample(30, 15).a, 0,
            "first offset alone must not count as the full chain");
        assert_eq!(routed.sample(15, 28).a, 0,
            "second offset alone must not count as the full chain");
    }

    #[test]
    fn multi_filter_chain_single_and_empty_groups_keep_original_output() {
        let none = frame(0, &[]);
        let parallel = frame(2, &[]);
        assert_eq!(none.data_as_u8_slice(), parallel.data_as_u8_slice());
        let single = frame(0, &[Filter::Offset { dx: 0.0, dy: 0.0 }]);
        assert_eq!(single.data_as_u8_slice(), none.data_as_u8_slice());
        assert!(single.sample(15, 15).a > 200);
        let mut painter = Painter::new(80, 80, RenderSettings { num_threads: 0, ..Default::default() });
        painter.begin(0, 0, 80, 80);
        painter.push_filter_chain(None, None, None, None, &[]);
        painter.push_filter_chain(None, None, None, None, &[
            Filter::Offset { dx: 1.0, dy: 2.0 },
            Filter::Offset { dx: 3.0, dy: 4.0 },
        ]);
        assert!(painter.filtered());
        assert_eq!(painter.groups, vec![0, 2]);
        painter.pop_filter_chain().unwrap();
        assert!(!painter.filtered());
        painter.pop_filter_chain().unwrap();
        assert!(painter.groups.is_empty());
        assert!(painter.pop_filter_chain().is_err(), "reject unbalanced group pop");
    }
}
