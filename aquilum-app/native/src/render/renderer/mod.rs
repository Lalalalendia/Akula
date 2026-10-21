mod pass;

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
    /// Time spent resetting and resizing Vello (including worker wake-up).
    pub reset: std::time::Duration,
    /// Masonry scene replay and Vello command encoding.
    pub replay: std::time::Duration,
    /// Flush of encoded work before rasterization.
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


#[cfg(not(feature = "installed"))]
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageSample {
    // Encoder includes mip build. Durations measure submitted CPU work only.
    pub scene_ms: f64,
    pub renderer_draw_ms: f64,
    pub output_present_ms: f64,
    // Windows developer-only WARP trace. CPU call times, not GPU completion.
    #[cfg(target_os = "windows")]
    pub output_map_ms: f64,
    #[cfg(target_os = "windows")]
    pub output_unmap_ms: f64,
    #[cfg(target_os = "windows")]
    pub output_copy_submit_ms: f64,
    #[cfg(target_os = "windows")]
    pub output_present_call_ms: f64,
    #[cfg(target_os = "windows")]
    pub output_copy_bytes: u64,
    #[cfg(target_os = "windows")]
    pub output_copy_rects: u64,
    #[cfg(target_os = "windows")]
    pub output_map_calls: u64,
    pub encode_ms: f64,
    pub vello_ms: f64,
    pub scroll_shift_ms: f64,
    pub mip_build_ms: f64,
    pub raster_pixels: u64,
    pub shifted_pixels: u64,
    pub raster_rects: usize,
    pub mip_hits: u64,
    pub mip_generated: u64,
}

#[cfg(not(feature = "installed"))]
pub(super) fn stages_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("AKULA_EXPERIMENTAL_FRAME_STAGES").ok().as_deref() == Some("1")
    })
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
    #[cfg(not(feature = "installed"))]
    pub(super) last_diagnostic_stages: StageSample,
    #[cfg(test)]
    pub(super) frame: (u16, u16, Vec<u32>),
    #[cfg(test)]
    pub(crate) last_stages: FrameStages,
    #[cfg(test)]
    pub shifts: usize,
    #[cfg(test)]
    pub rastered: Vec<(u16, u16, u16, u16)>,
    #[cfg(test)]
    pub moved: Vec<(u16, u16, u16, u16, i32)>,
    #[cfg(test)]
    pub causes: Vec<masonry::kurbo::Rect>,
}

/// A single small final dirty rectangle costs less with a zero-worker Vello
/// context than with thread handoff. Large or fragmented damage still uses
/// the original parallel context. See the measured #218 RSS/latency gate and
/// the fixed real-glyph guard96 fidelity gate #260.
const SMALL_SERIAL_MAX_PIXELS: u64 = 350_000;

/// A rollout escape hatch; a value of "0" disables the extra serial context.
/// The former experimental variable stays supported for existing deployments.
fn single_rect_route_enabled(current: Option<&str>, legacy: Option<&str>) -> bool {
    current != Some("0") && legacy != Some("0")
}

impl Renderer {
    pub fn new(threads: u16) -> Self {
        let current = std::env::var("AKULA_VELLO_SINGLE_RECT").ok();
        let legacy = std::env::var("AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT").ok();
        let adaptive = single_rect_route_enabled(current.as_deref(), legacy.as_deref());
        Self::new_with_serial_route(threads, adaptive)
    }

    fn new_with_serial_route(threads: u16, adaptive: bool) -> Self {
        let mut renderer = Renderer {
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
            #[cfg(not(feature = "installed"))]
            last_diagnostic_stages: StageSample::default(),
            #[cfg(test)]
            frame: (0, 0, Vec::new()),
            #[cfg(test)]
            last_stages: FrameStages::default(),
            #[cfg(test)]
            shifts: 0,
            #[cfg(test)]
            rastered: Vec::new(),
            #[cfg(test)]
            moved: Vec::new(),
            #[cfg(test)]
            causes: Vec::new(),
        };
        if adaptive {
            renderer.painter.enable_serial_for_single_damage(SMALL_SERIAL_MAX_PIXELS);
        }
        renderer
    }

    /// In benchmarks, preserve a true fixed-worker control even when small
    /// single-rectangle routing is the product default.
    #[cfg(test)]
    pub(crate) fn new_fixed_for_test(threads: u16) -> Self {
        Self::new_with_serial_route(threads, false)
    }

    /// Construct a renderer with an explicitly detected SIMD level for parity/perf
    /// experiments. Production still uses the conservative paint::settings cap.
    #[cfg(test)]
    pub(crate) fn with_simd_level_for_test(threads: u16, level: vello_cpu::Level) -> Self {
        let mut renderer = Self::new_fixed_for_test(threads);
        renderer.painter = Painter::new(1, 1, vello_cpu::RenderSettings { num_threads: threads, level });
        renderer
    }

    /// Test-only explicit dual-dispatch route for a fixed-policy benchmark control.
    #[cfg(test)]
    pub(crate) fn enable_dual_dispatch_for_test(&mut self, max_serial_pixels: u64) {
        self.painter.enable_dual_dispatch_for_test(max_serial_pixels);
    }

    #[cfg(test)]
    pub(crate) fn dual_dispatch_switches_for_test(&self) -> usize {
        self.painter.test_route_switches
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

    #[cfg(not(feature = "installed"))]
    pub fn diagnostic_stages(&self) -> StageSample {
        self.last_diagnostic_stages
    }

    #[cfg(not(feature = "installed"))]
    pub fn diagnostic_stages_enabled() -> bool {
        stages_enabled()
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

#[cfg(test)]
mod small_rect_policy_tests {
    use super::{Renderer, single_rect_route_enabled};

    #[test]
    fn adaptive_constructor_switches_only_when_enabled() {
        let small = [(0, 0, 100, 100)];
        let mut adaptive = Renderer::new_with_serial_route(2, true);
        adaptive.painter.route_for_damage(&small);
        assert_eq!(adaptive.dual_dispatch_switches_for_test(), 1);
        adaptive.painter.route_for_damage(&[(0, 0, 1000, 600)]);
        assert_eq!(adaptive.dual_dispatch_switches_for_test(), 2);

        let mut fixed = Renderer::new_with_serial_route(2, false);
        fixed.painter.route_for_damage(&small);
        assert_eq!(fixed.dual_dispatch_switches_for_test(), 0);

        let mut zero_workers = Renderer::new_with_serial_route(0, true);
        zero_workers.painter.route_for_damage(&small);
        assert_eq!(zero_workers.dual_dispatch_switches_for_test(), 0);
    }

    #[test]
    fn adaptive_small_rect_default_has_explicit_rollback() {
        assert!(single_rect_route_enabled(None, None));
        assert!(single_rect_route_enabled(Some("1"), None));
        assert!(single_rect_route_enabled(None, Some("1")));
        assert!(!single_rect_route_enabled(Some("0"), None));
        assert!(!single_rect_route_enabled(None, Some("0")));
        assert!(!single_rect_route_enabled(Some("1"), Some("0")));
        assert!(!single_rect_route_enabled(Some("0"), Some("1")));
    }
}
