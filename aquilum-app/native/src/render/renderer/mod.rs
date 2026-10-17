mod pass;
#[cfg(test)]
mod adjacent;

use masonry::app::{VisualLayerKind, VisualLayerPlan};
use masonry::imaging::record::{Scene, replay_transformed};
use masonry::imaging::{PaintSink, Painter as Sink};
use masonry::kurbo::{Affine, Rect};
use masonry::peniko::Color;

use std::collections::HashMap;

use super::damage::Tracker;
use super::layer::{Cache, Layer};
use super::live;
use super::overlay::Overlay;
use super::paint::{PaintError, Painter};
use super::sprite::Sprite;
use super::stamp::Stamps;
use crate::output::{Area, Canvas};

pub(super) struct Frame<'a> {
    width: u16,
    height: u16,
    scale: f64,
    background: Color,
    base: &'a Scene,
    overlays: Vec<(&'a Scene, Affine)>,
    pub(super) fading: Option<(&'a Scene, Affine, super::fade::Fade, u64)>,
}

impl Frame<'_> {
    pub(super) fn paint_into(&self, sink: &mut dyn PaintSink) {
        Sink::new(sink).fill_rect(Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height)), self.background);
        let scale = Affine::scale(self.scale);
        replay_transformed(self.base, sink, scale);
        for (scene, transform) in &self.overlays {
            replay_transformed(scene, sink, scale * *transform);
        }
    }
}

/// Test-only wall-clock breakdown of an actual Akula frame. It is not emitted
/// in release builds and does not alter rasterizer or damage behavior.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameStages {
    pub record: std::time::Duration,
    pub detect: std::time::Duration,
    pub encode: std::time::Duration,
    /// Includes RenderContext reset_and_resize + worker broadcast/barrier and local setup.
    pub reset: std::time::Duration,
    /// Replaying recorded Masonry layers into Vello on the CPU.
    pub replay: std::time::Duration,
    /// Vello flush, including the completion of queued worker operations.
    pub flush: std::time::Duration,
    pub raster: std::time::Duration,
    /// Pixels scheduled for rasterization after merging and live-layer updates.
    /// Areas may overlap, so this is workload area, not distinct-screen coverage.
    pub raster_pixels: u64,
    pub shifted_pixels: u64,
    pub screen_pixels: u64,
    pub raster_rects: usize,
    pub damage: super::damage::DamageStats,
}

pub struct Renderer {
    pub(super) painter: Painter,
    pub(super) scratch: Vec<u32>,
    tracker: Tracker,
    pub(super) params: Option<(u16, u16, f64, Color)>,
    pub(super) live: Vec<Layer>,
    pub(super) caches: HashMap<u64, Cache>,
    pub(super) frames: HashMap<u64, i64>,
    pub(super) overlay: Overlay,
    pub(super) stamps: Stamps,
    pub(super) faded: Option<super::fade::Cached>,
    sprites: Vec<Sprite>,
    sprite_stamps: Stamps,
    sprite_changes: Option<Area>,
    stamped: Option<Vec<Sprite>>,
    shifted: Vec<Area>,
    #[cfg(test)]
    pub(super) frame: (u16, u16, Vec<u32>),
    #[cfg(test)]
    pub(crate) last_stages: FrameStages,
    #[cfg(test)]
    pub(crate) split_full_frame_strips: Option<u16>,
    #[cfg(test)]
    pub(crate) coalesce_adjacent_for_test: bool,
    #[cfg(test)]
    pub shifts: usize,
    #[cfg(test)]
    pub rastered: Vec<(u16, u16, u16, u16)>,
    #[cfg(test)]
    pub moved: Vec<(u16, u16, u16, u16, i32)>,
    #[cfg(test)]
    pub causes: Vec<masonry::kurbo::Rect>,
}

impl Renderer {
    pub fn new(threads: u16) -> Self {
        Renderer {
            painter: Painter::new(1, 1, super::paint::settings(threads)),
            scratch: Vec::new(),
            tracker: Tracker::new(),
            params: None,
            live: Vec::new(),
            caches: HashMap::new(),
            frames: HashMap::new(),
            overlay: Overlay::default(),
            stamps: Stamps::default(),
            faded: None,
            sprites: Vec::new(),
            sprite_stamps: Stamps::default(),
            sprite_changes: None,
            stamped: None,
            shifted: Vec::new(),
            #[cfg(test)]
            frame: (0, 0, Vec::new()),
            #[cfg(test)]
            last_stages: FrameStages::default(),
            #[cfg(test)]
            split_full_frame_strips: None,
            #[cfg(test)]
            coalesce_adjacent_for_test: false,
            #[cfg(test)]
            shifts: 0,
            #[cfg(test)]
            rastered: Vec::new(),
            #[cfg(test)]
            moved: Vec::new(),
            #[cfg(test)]
            causes: Vec::new(),
        }
    }

    /// Construct a renderer with an explicitly detected SIMD level for parity/perf
    /// experiments. Production still uses the conservative paint::settings cap.
    #[cfg(test)]
    pub(crate) fn with_simd_level_for_test(threads: u16, level: vello_cpu::Level) -> Self {
        let mut renderer = Self::new(threads);
        renderer.painter = Painter::new(1, 1, vello_cpu::RenderSettings { num_threads: threads, level });
        renderer
    }

    /// Test-only dual-dispatch prototype; production renderer is unchanged.
    #[cfg(test)]
    pub(crate) fn enable_dual_dispatch_for_test(&mut self, max_serial_pixels: u64) {
        self.painter.enable_dual_dispatch_for_test(max_serial_pixels);
    }

    #[cfg(test)]
    pub(crate) fn dual_dispatch_switches_for_test(&self) -> usize {
        self.painter.test_route_switches
    }

    /// Split an otherwise full redraw into nonoverlapping strips. This is a
    /// test-only falsifier for rectangle-local worker routing, not UI behavior.
    #[cfg(test)]
    pub(crate) fn split_full_frame_for_test(&mut self, strips: u16) {
        assert!((2..=16).contains(&strips), "supported full-frame strip counts: 2..16");
        self.split_full_frame_strips = Some(strips);
    }

    /// Route all rectangles of a frame by their *aggregate* scheduled pixels.
    #[cfg(test)]
    pub(crate) fn enable_frame_budget_for_test(&mut self) {
        self.painter.enable_frame_budget_for_test();
    }

    /// Opt-in test-only batching of full-edge adjacent disjoint regions.
    /// Never changes production damage or its pixel footprint.
    #[cfg(test)]
    pub(crate) fn enable_adjacent_coalescing_for_test(&mut self) {
        self.coalesce_adjacent_for_test = true;
    }

    pub fn toggle_overlay(&mut self) {
        self.overlay.on = !self.overlay.on;
        self.overlay.pending = true;
    }

    pub fn overlay_busy(&self) -> bool {
        self.overlay.on || self.overlay.pending
    }

    pub(super) fn frame<'a>(&self, layers: &'a VisualLayerPlan) -> Option<Frame<'a>> {
        let (width, height, scale, background) = self.params?;
        let Some(VisualLayerKind::Scene(base)) = layers.root_layer().map(|l| &l.kind) else { return None };
        let mut fading = None;
        let mut overlays = Vec::new();
        for layer in layers.overlay_layers() {
            let VisualLayerKind::Scene(scene) = &layer.kind else { continue };
            let id = layer.widget_id.to_raw();
            match super::fade::get(id) {
                Some(fade) if fading.is_none() => fading = Some((scene, layer.transform, fade, id)),
                _ => overlays.push((scene, layer.transform)),
            }
        }
        Some(Frame { width, height, scale, background, base, overlays, fading })
    }

    pub fn trim(&mut self) {
        self.painter.trim();
        self.stamps.forget();
        self.stamps.trim();
        self.sprite_stamps.forget();
        self.sprite_stamps.trim();
        self.stamped = None;
        self.faded = None;
        self.caches.clear();
        self.frames.clear();
        self.scratch = Vec::new();
        self.tracker.forget();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, layers: &VisualLayerPlan, width: u16, height: u16, scale: f64, background: Color, full: bool, canvas: &mut dyn Canvas) -> Result<Vec<Area>, PaintError> {
        self.params = Some((width, height, scale, background));
        if full {
            self.stamps.forget();
            self.sprite_stamps.forget();
            self.stamped = None;
        }
        let seconds = live::seconds();
        let restored = self.stamps.restore(canvas);
        let drawn = self.paint(layers, full, canvas, seconds)?;
        let live = self.composite(layers, canvas, seconds)?;
        self.put_sprites(canvas);
        let sprites = self.sprite_changes.take();
        let faded = self.fade(layers, canvas)?;
        let shown = self.decorate(canvas);
        Ok(drawn.into_iter().chain([restored, live, sprites, faded, shown].into_iter().flatten()).collect())
    }

    pub fn live(&mut self, layers: &VisualLayerPlan, canvas: &mut dyn Canvas) -> Result<Vec<Area>, PaintError> {
        let seconds = live::seconds();
        let restored = self.stamps.restore(canvas);
        let touched = self.due_areas(seconds);
        self.lift_sprites(canvas, &touched);
        let live = self.composite(layers, canvas, seconds)?;
        self.put_sprites(canvas);
        let faded = self.fade(layers, canvas)?;
        let shown = self.decorate(canvas);
        Ok([restored, live, faded, shown].into_iter().flatten().collect())
    }
}
