pub(super) mod convert;
mod cull;
mod error;
mod mask;
#[cfg(any(test, not(feature = "installed")))]
mod mipmap;
mod runs;
mod sink;

use std::collections::VecDeque;

use masonry::kurbo::{Affine, Rect};
use vello_cpu::{RenderContext, RenderSettings, Resources};

pub use error::PaintError;
use mask::CachedMask;

pub fn settings(threads: u16) -> RenderSettings {
    let level = RenderSettings::default().level;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    let level = level.as_sse4_2().map_or(level, vello_cpu::Level::Sse4_2);
    RenderSettings { num_threads: threads, level }
}

const TOLERANCE: f64 = 0.1;

pub struct Painter {
    pub(super) ctx: RenderContext,
    pub(super) resources: Resources,
    pub(super) settings: RenderSettings,
    pub(super) error: Option<PaintError>,
    pub(super) clip_depth: u32,
    pub(super) groups: Vec<bool>,
    pub(super) mask_cache: VecDeque<CachedMask>,
    pub(super) frame: Rect,
    pub(super) shift: Affine,
    pub(super) images: convert::Images,
    pub(super) culled: u32,
    pub(super) seconds: f64,
    pub(super) regions: Vec<Rect>,
    /// Optional persistent zero-worker Vello context sharing Resources with the
    /// parallel renderer. Allocated by adaptive small-damage routing, or lazily
    /// when a filtered scene requires the serial-only filter implementation.
    serial_ctx: Option<RenderContext>,
    serial_active: bool,
    serial_limit_px: Option<u64>,
    #[cfg(test)]
    pub(super) test_route_switches: usize,
}

impl Painter {
    #[cfg(not(feature = "installed"))]
    pub(crate) fn mip_diagnostic_counters(&self) -> (u128, u64, u64) {
        self.images.diagnostic_counters()
    }

    pub fn new(width: u16, height: u16, settings: RenderSettings) -> Self {
        Painter {
            ctx: RenderContext::new_with(width, height, settings),
            resources: Resources::new(),
            settings,
            error: None,
            clip_depth: 0,
            groups: Vec::new(),
            mask_cache: VecDeque::new(),
            frame: Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            shift: Affine::IDENTITY,
            images: convert::Images::default(),
            culled: 0,
            seconds: 0.0,
            regions: Vec::new(),
            serial_ctx: None,
            serial_active: false,
            serial_limit_px: None,
            #[cfg(test)]
            test_route_switches: 0,
        }
    }

    pub fn trim(&mut self) {
        self.ctx = RenderContext::new_with(1, 1, self.settings);
        self.serial_ctx = self.serial_limit_px.map(|_| RenderContext::new_with(
            1, 1, RenderSettings { num_threads: 0, ..self.settings },
        ));
        self.serial_active = false;
        #[cfg(test)]
        {
            self.test_route_switches = 0;
        }
        self.resources = Resources::new();
        self.images = convert::Images::default();
        self.mask_cache = VecDeque::new();
    }

    pub fn width(&self) -> u16 {
        self.ctx.width()
    }

    pub fn height(&self) -> u16 {
        self.ctx.height()
    }

    /// Experimental opt-in: retain one serial context next to the existing
    /// parallel context. Both use the same glyph/image Resources; do not create
    /// Rayon pools or caches while switching frames. Does nothing for 0 workers.
    pub(crate) fn enable_serial_for_single_damage(&mut self, max_serial_pixels: u64) {
        if self.settings.num_threads == 0 || max_serial_pixels == 0 {
            return;
        }
        if self.serial_ctx.is_none() {
            self.serial_ctx = Some(RenderContext::new_with(
                1, 1, RenderSettings { num_threads: 0, ..self.settings },
            ));
        }
        self.serial_limit_px = Some(max_serial_pixels);
        self.serial_active = false;
        #[cfg(test)]
        {
            self.test_route_switches = 0;
        }
    }

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

    /// Existing #166 test caller; routing now has the single-rectangle guard.
    #[cfg(test)]
    pub(crate) fn enable_dual_dispatch_for_test(&mut self, max_serial_pixels: u64) {
        assert!(self.settings.num_threads > 0, "a parallel baseline is required");
        assert!(self.serial_ctx.is_none(), "route experiment enabled twice");
        self.enable_serial_for_single_damage(max_serial_pixels);
    }

    /// Begin the first damaged region of a frame, advancing image cache age
    /// exactly once regardless of how many separate dirty regions follow.
    pub fn begin(&mut self, x: u16, y: u16, width: u16, height: u16) {
        self.images.next_frame();
        self.begin_region(x, y, width, height);
    }

    /// Begin an additional region of the same draw pass. Replaying a second
    /// rectangle must not age/evict the image mipmaps used by the first.
    pub(crate) fn begin_region(&mut self, x: u16, y: u16, width: u16, height: u16) {
        if self.ctx.width() != width || self.ctx.height() != height {
            self.ctx.reset_and_resize(width, height);
            self.mask_cache.clear();
        } else {
            self.ctx.reset();
        }
        self.frame = Rect::new(f64::from(x), f64::from(y), f64::from(x) + f64::from(width), f64::from(y) + f64::from(height));
        self.shift = Affine::translate((-f64::from(x), -f64::from(y)));
        self.error = None;
        self.clip_depth = 0;
        self.groups.clear();
        self.culled = 0;
    }

    pub(super) fn set_transform(&mut self, transform: Affine) {
        self.ctx.set_transform(self.shift * transform);
    }

    pub fn finish(&mut self) -> Result<(), PaintError> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if self.clip_depth != 0 {
            return Err(PaintError::Unbalanced("клип не закрыт"));
        }
        if !self.groups.is_empty() {
            return Err(PaintError::Unbalanced("группа не закрыта"));
        }
        self.ctx.flush();
        Ok(())
    }

    pub(super) fn set_error_once(&mut self, err: PaintError) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }
}

#[cfg(test)]
mod adaptive_route_tests {
    use super::*;

    #[test]
    fn adaptive_serial_only_for_single_small_damage() {
        let mut painter = Painter::new(1, 1, settings(2));
        let small = [(0, 0, 100, 100)];
        painter.route_for_damage(&small);
        assert!(!painter.serial_active, "the experimental route must default off");

        painter.enable_serial_for_single_damage(350_000);
        assert!(painter.serial_ctx.is_some());
        painter.route_for_damage(&small);
        assert!(painter.serial_active);
        assert_eq!(painter.test_route_switches, 1);

        // Two individually tiny touching strips must NEVER trigger the
        // single-rectangle fast path.
        painter.route_for_damage(&[(0, 0, 100, 100), (100, 0, 100, 100)]);
        assert!(!painter.serial_active);
        painter.route_for_damage(&[(0, 0, 100, 100), (0, 0, 100, 100)]);
        assert!(!painter.serial_active, "overlap is also fragmented damage");
        painter.route_for_damage(&[(0, 0, 1000, 600)]);
        assert!(!painter.serial_active);
        painter.route_for_damage(&[]);
        assert!(!painter.serial_active);
        painter.route_for_damage(&[(0, 0, 0, 100)]);
        assert!(!painter.serial_active);

        painter.route_for_damage(&small);
        assert!(painter.serial_active);
        painter.trim();
        assert!(!painter.serial_active, "trim must restore the parallel primary context");
        assert!(painter.serial_ctx.is_some(), "trim retains experimental mode");
        painter.route_for_damage(&small);
        assert!(painter.serial_active, "routing must still work after trim");
    }


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

    #[test]
    fn zero_worker_renderer_never_allocates_a_companion() {
        let mut painter = Painter::new(1, 1, settings(0));
        painter.enable_serial_for_single_damage(350_000);
        assert!(painter.serial_ctx.is_none());
        painter.route_for_damage(&[(0, 0, 100, 100)]);
        assert!(!painter.serial_active);
    }
}
