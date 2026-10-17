pub(super) mod convert;
mod cull;
mod error;
mod mask;
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
    #[cfg(test)]
    test_serial_ctx: Option<RenderContext>,
    #[cfg(test)]
    test_serial_active: bool,
    #[cfg(test)]
    test_route_limit_px: Option<u64>,
    #[cfg(test)]
    pub(super) test_route_switches: usize,
}

impl Painter {
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
            #[cfg(test)]
            test_serial_ctx: None,
            #[cfg(test)]
            test_serial_active: false,
            #[cfg(test)]
            test_route_limit_px: None,
            #[cfg(test)]
            test_route_switches: 0,
        }
    }

    pub fn trim(&mut self) {
        self.ctx = RenderContext::new_with(1, 1, self.settings);
        #[cfg(test)]
        {
            self.test_serial_ctx = self.test_route_limit_px.map(|_| RenderContext::new_with(
                1, 1, RenderSettings { num_threads: 0, ..self.settings },
            ));
            self.test_serial_active = false;
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

    /// Experimental context routing: keep both dispatchers hot, sharing
    /// image/glyph resources and masks. Never enabled in production builds.
    #[cfg(test)]
    pub(crate) fn enable_dual_dispatch_for_test(&mut self, max_serial_pixels: u64) {
        assert!(self.settings.num_threads > 0, "a parallel baseline is required");
        assert!(self.test_serial_ctx.is_none(), "route experiment enabled twice");
        self.test_serial_ctx = Some(RenderContext::new_with(
            1, 1, RenderSettings { num_threads: 0, ..self.settings },
        ));
        self.test_route_limit_px = Some(max_serial_pixels);
        self.test_serial_active = false;
        self.test_route_switches = 0;
    }

    /// Select the persistent dispatcher *before* encoding a fresh damage rect.
    /// RenderContext is always flushed in finish() before this can be called.
    #[cfg(test)]
    pub(crate) fn route_for_test(&mut self, width: u16, height: u16) {
        let Some(limit) = self.test_route_limit_px else { return };
        let use_serial = u64::from(width) * u64::from(height) <= limit;
        if use_serial != self.test_serial_active {
            let alternate = self.test_serial_ctx.as_mut().expect("route companion context");
            std::mem::swap(&mut self.ctx, alternate);
            self.test_serial_active = use_serial;
            self.test_route_switches += 1;
        }
    }

    pub fn begin(&mut self, x: u16, y: u16, width: u16, height: u16) {
        if self.ctx.width() != width || self.ctx.height() != height {
            self.ctx.reset_and_resize(width, height);
            self.mask_cache.clear();
        } else {
            self.ctx.reset();
        }
        self.images.next_frame();
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
