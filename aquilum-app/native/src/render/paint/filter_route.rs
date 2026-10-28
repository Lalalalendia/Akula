//! Filter-safe serial routing and optional small-damage routing.

use super::*;

impl Painter {
    /// Route once for the ENTIRE frame, before encoding its first dirty region.
    /// Never route fragmented or empty damage to serial even if each individual
    /// rectangle is small: that case was falsified by Akula #173/#187.
    /// Keep the prior adaptive-route test surface independent of filter
    /// admission. Production calls route_for_scene after damage preflight.
    #[cfg(test)]
    pub(crate) fn route_for_damage(&mut self, areas: &[(u16, u16, u16, u16)]) {
        self.route_for_scene(areas, false);
    }

    /// A filtered scene must use a zero-worker context before ANY commands are
    /// encoded. MultiThreadedDispatcher::push_layer panics for filter_data.
    /// This safety route also applies with AKULA_VELLO_SINGLE_RECT=0.
    pub(crate) fn route_for_scene(&mut self, areas: &[(u16, u16, u16, u16)], has_filters: bool) {
        let force_serial = has_filters && self.settings.num_threads > 0 && !areas.is_empty();
        let adaptive_serial = self.serial_limit_px.is_some_and(|limit| match areas {
            &[(_, _, w, h)] if w > 0 && h > 0 => u64::from(w) * u64::from(h) <= limit,
            _ => false,
        });
        let use_serial = force_serial || adaptive_serial;
        if force_serial && self.serial_ctx.is_none() {
            self.serial_ctx = Some(RenderContext::new_with(
                1, 1, RenderSettings { num_threads: 0, ..self.settings },
            ));
        }
        if use_serial != self.serial_active {
            // The previous frame called finish() before rasterizing. Each
            // context retains its own dispatcher and allocations when inactive.
            let alternate = self.serial_ctx.as_mut().expect("serial companion context");
            std::mem::swap(&mut self.ctx, alternate);
            self.serial_active = use_serial;
            #[cfg(test)]
            {
                self.test_route_switches += 1;
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_route_is_serial_even_when_adaptive_is_disabled() {
        let mut painter = Painter::new(1, 1, settings(2));
        let large = [(0, 0, 800, 800), (0, 800, 800, 100)];
        assert!(painter.serial_ctx.is_none(), "no eager extra thread pool");
        painter.route_for_scene(&large, true);
        assert!(painter.serial_active, "filtered frames must use serial Vello");
        assert_eq!(painter.ctx.render_settings().num_threads, 0);
        assert_eq!(painter.test_route_switches, 1);
        painter.route_for_scene(&large, true);
        assert_eq!(painter.test_route_switches, 1, "keep the serial context");
        painter.route_for_scene(&large, false);
        assert!(!painter.serial_active, "restore configured parallel routing");
        assert_eq!(painter.ctx.render_settings().num_threads, 2);
        painter.route_for_scene(&large, true);
        assert_eq!(painter.ctx.render_settings().num_threads, 0);
        painter.trim();
        assert!(!painter.serial_active, "trim must reset the safety route");
        assert!(painter.serial_ctx.is_none(), "no unnecessary permanent companion when disabled");
        painter.route_for_scene(&large, true);
        assert!(painter.serial_active, "safety route recreates after trim");
    }

    #[test]
    fn filter_layer_survives_serial_fallback_without_panic() {
        // Two damage rects would normally force the multi-worker path. The
        // filtered group instead uses the actual zero-worker render context.
        let mut painter = Painter::new(1, 1, settings(2));
        painter.route_for_scene(&[(0, 0, 40, 40), (40, 0, 40, 40)], true);
        painter.begin(0, 0, 80, 40);
        let filter = convert::filters_to_vello(&[masonry::imaging::Filter::Offset {
            dx: 0.0,
            dy: 0.0,
        }]).expect("offset filter");
        painter.ctx.push_filter_layer(filter);
        painter.ctx.set_paint(vello_cpu::color::palette::css::RED);
        painter.ctx.fill_rect(&Rect::new(8.0, 8.0, 32.0, 32.0));
        painter.ctx.pop_layer();
        painter.finish().expect("serial filter flush");
        let mut pixmap = vello_cpu::Pixmap::new(80, 40);
        painter.ctx.render_with(
            &mut pixmap,
            &mut painter.resources,
            vello_cpu::RasterizerSettings::default(),
        );
        assert!(
            pixmap.data_as_u8_slice().chunks_exact(4).any(|px| px[0] > 0 && px[3] > 0),
            "the filter must draw, not merely avoid panicking"
        );
    }

}
