#[cfg(test)]
use std::time::Duration;
#[cfg(any(test, not(feature = "installed")))]
use std::time::Instant;

use masonry::app::VisualLayerPlan;
use masonry::kurbo::Rect;
use vello_cpu::{PixelFormat, PixmapMut, RasterizerSettings, TargetInit};

use super::Renderer;
use crate::output::{Area, Canvas, Target};
use crate::render::damage::Damage;
use crate::render::layer::{Layer, area, area_of};
use crate::render::overlay::Mark;
use crate::render::paint::PaintError;
use crate::render::sprite::{self, Sprite};
use crate::render::stamp::covering;

impl Renderer {
    pub(super) fn lift_sprites(&mut self, canvas: &mut dyn Canvas, touched: &[Area]) {
        let visible: Vec<Sprite> = self.sprites.iter().filter(|s| !s.covered).copied().collect();
        if !self.overlay.on && self.stamped.as_ref() == Some(&visible) && self.sprite_stamps.untouched(touched) {
            return;
        }
        self.sprite_stamps.restore(canvas);
        self.stamped = None;
    }

    pub(super) fn put_sprites(&mut self, canvas: &mut dyn Canvas) {
        if self.stamped.is_some() {
            return;
        }
        sprite::stamp(&self.sprites, canvas, &mut self.sprite_stamps);
        self.stamped = Some(self.sprites.iter().filter(|s| !s.covered).copied().collect());
    }

    pub(super) fn paint(&mut self, layers: &VisualLayerPlan, full: bool, canvas: &mut dyn Canvas, seconds: f64) -> Result<Vec<Area>, PaintError> {
        #[cfg(not(feature = "installed"))]
        let stage_on = super::stages_enabled();
        #[cfg(not(feature = "installed"))]
        let mut stage = super::StageSample::default();
        #[cfg(not(feature = "installed"))]
        {
            self.last_diagnostic_stages = super::StageSample::default();
        }
        let Some((width, height, scale, _)) = self.params else {
            self.lift_sprites(canvas, &[]);
            return Ok(Vec::new());
        };
        let Some(frame) = self.frame(layers) else {
            self.lift_sprites(canvas, &[]);
            return Ok(Vec::new());
        };
        self.painter.regions = crate::render::scroll::snapshot().iter().map(|r| r.viewport.scale_from_origin(scale)).collect();
        #[cfg(test)]
        let record_time;
        #[cfg(test)]
        let detect_time;
        let damage = {
            let _zone = aq_trace::zone("поиск изменений");
            #[cfg(test)]
            let record_started = Instant::now();
            self.tracker.begin(width, height, scale);
            frame.paint_into(&mut self.tracker);
            #[cfg(test)]
            {
                record_time = record_started.elapsed();
            }
            #[cfg(test)]
            let detect_started = Instant::now();
            let damage = self.tracker.damage();
            #[cfg(test)]
            {
                detect_time = detect_started.elapsed();
            }
            damage
        };
        #[cfg(test)]
        let mut stages = super::FrameStages {
            record: record_time,
            detect: detect_time,
            damage: self.tracker.last_stats,
            ..Default::default()
        };
        let mut damage = if full { Damage { areas: vec![(0, 0, width, height)], moves: Vec::new() } } else { damage };
        #[cfg(test)]
        if std::env::var_os("AQ_TEST_FORCE_FULL_ON_SCROLL").is_some() && !damage.moves.is_empty() {
            // Test-only falsifier: when a scroll would normally use pixel
            // copies, re-encode and re-rasterize the *whole* screen. This
            // isolates clipped-region painter issues from persistent context.
            damage = Damage { areas: vec![(0, 0, width, height)], moves: Vec::new() };
        }
        #[cfg(test)]
        if std::env::var_os("AQ_TEST_REDRAW_SCROLL_VIEWPORT").is_some() {
            // Only for the release test harness: replace pixel copies with
            // full re-rasterization of the same scrolled viewport. This
            // falsifies stale-pixel reuse without altering release behavior.
            damage.areas.extend(damage.moves.iter().map(|m| (m.x, m.y, m.w, m.h)));
            damage.moves.clear();
        }
        #[cfg(test)]
        if let Ok(guard) = std::env::var("AQ_TEST_SCROLL_EDGE_GUARD") {
            let guard = guard.parse::<u16>().expect("edge guard must be u16");
            for m in &damage.moves {
                let band = guard.min(m.h);
                damage.areas.push((m.x, m.y, m.w, band));
                damage.areas.push((m.x, m.y.saturating_add(m.h - band), m.w, band));
            }
        }
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let layers = self.tracker.live.iter().filter_map(|&(id, rect, transform, shape)| Some(Layer { id, area: area_of(rect, bounds)?, transform, shape })).collect();
        self.refresh_live(layers, &mut damage, full, seconds);
        #[cfg(not(feature = "installed"))]
        if stage_on {
            // Damage can contain overlapping rectangles. Count scheduled
            // raster work, not distinct screen coverage.
            stage.raster_pixels = damage.areas.iter()
                .map(|&(_, _, w, h)| u64::from(w) * u64::from(h)).sum();
            stage.shifted_pixels = damage.moves.iter()
                .map(|m| u64::from(m.w) * u64::from(m.h)).sum();
            stage.raster_rects = damage.areas.len();
        }
        #[cfg(test)]
        {
            // Measure the *final* scheduled raster and row-shift workload.
            // Use u64 so large physical frames cannot overflow the counter.
            stages.screen_pixels = u64::from(width) * u64::from(height);
            stages.raster_rects = damage.areas.len();
            stages.raster_pixels = damage.areas.iter().map(|&(_, _, w, h)| u64::from(w) * u64::from(h)).sum();
            stages.shifted_pixels = damage.moves.iter().map(|m| u64::from(m.w) * u64::from(m.h)).sum();
        }
        let sprites = std::mem::take(&mut self.tracker.sprites);
        self.sprite_changes = sprite::changes(&self.sprites, &sprites);
        self.sprites = sprites;
        self.shifted = damage.moves.iter().map(|m| area((m.x, m.y, m.w, m.h))).collect();
        #[cfg(test)]
        {
            self.shifts += damage.moves.len();
            self.rastered = damage.areas.clone();
            self.moved = damage.moves.iter().map(|m| (m.x, m.y, m.w, m.h, m.dy)).collect();
            self.causes = std::mem::take(&mut self.tracker.causes);
        }
        let dirty: Vec<Area> = damage.areas.iter().copied().chain(damage.moves.iter().map(|m| (m.x, m.y, m.w, m.h))).map(area).collect();
        let touched: Vec<Area> = dirty.iter().copied().chain(self.due_areas(seconds)).collect();
        self.lift_sprites(canvas, &touched);
        if dirty.is_empty() {
            #[cfg(test)]
            {
                self.last_stages = stages;
            }
            return Ok(dirty);
        }
        for &rect in &damage.areas {
            self.overlay.flash(area(rect), Mark::Raster);
        }
        let painter = &mut self.painter;
        painter.seconds = seconds;
        painter.route_for_scene(&damage.areas, self.tracker.has_filters);
        #[cfg(not(feature = "installed"))]
        let mip_before = stage_on.then(|| painter.mip_diagnostic_counters());
        let mut result = Ok(());
        #[cfg(test)]
        let mut encoding = Duration::ZERO;
        #[cfg(test)]
        let mut reset_time = Duration::ZERO;
        #[cfg(test)]
        let mut replay_time = Duration::ZERO;
        #[cfg(test)]
        let mut flush_time = Duration::ZERO;
        #[cfg(test)]
        let mut rasterizing = Duration::ZERO;
        canvas.with_target(&mut |Target { pixels, width, height }| {
            #[cfg(not(feature = "installed"))]
            let shift_started = stage_on.then(Instant::now);
            for shift in &damage.moves {
                let _zone = aq_trace::zone("сдвиг прокрутки");
                crate::render::scroll::shift_rows(pixels, usize::from(width), *shift);
            }
            #[cfg(not(feature = "installed"))]
            if let Some(start) = shift_started {
                stage.scroll_shift_ms += start.elapsed().as_secs_f64() * 1000.0;
            }
            for (region_index, &(x, y, w, h)) in damage.areas.iter().enumerate() {
                #[cfg(test)]
                let encoding_started = Instant::now();
                #[cfg(not(feature = "installed"))]
                let stage_encoding_started = stage_on.then(Instant::now);
                {
                    let _zone = aq_trace::zone("кодирование");
                    #[cfg(test)]
                    let reset_started = Instant::now();
                    if region_index == 0 {
                        painter.begin(x, y, w, h);
                    } else {
                        painter.begin_region(x, y, w, h);
                    }
                    #[cfg(test)]
                    {
                        reset_time += reset_started.elapsed();
                    }
                    #[cfg(test)]
                    let replay_started = Instant::now();
                    frame.paint_into(painter);
                    #[cfg(test)]
                    {
                        replay_time += replay_started.elapsed();
                    }
                    #[cfg(test)]
                    let flush_started = Instant::now();
                    if let Err(err) = painter.finish() {
                        result = Err(err);
                        return;
                    }
                    #[cfg(test)]
                    {
                        flush_time += flush_started.elapsed();
                    }
                }
                #[cfg(test)]
                {
                    encoding += encoding_started.elapsed();
                }
                #[cfg(not(feature = "installed"))]
                if let Some(start) = stage_encoding_started {
                    stage.encode_ms += start.elapsed().as_secs_f64() * 1000.0;
                }
                #[cfg(test)]
                let raster_started = Instant::now();
                #[cfg(not(feature = "installed"))]
                let stage_raster_started = stage_on.then(Instant::now);
                let _zone = aq_trace::zone("растеризация");
                let pixmap = PixmapMut::new(width, height, bytemuck::cast_slice_mut(&mut *pixels)).expect("холст вывода");
                let settings = RasterizerSettings { target_init: TargetInit::SrcOver, pixel_format: PixelFormat::Bgra8, offset: (x, y) };
                painter.ctx.render_with(pixmap, &mut painter.resources, settings);
                #[cfg(test)]
                {
                    rasterizing += raster_started.elapsed();
                }
                #[cfg(not(feature = "installed"))]
                if let Some(start) = stage_raster_started {
                    stage.vello_ms += start.elapsed().as_secs_f64() * 1000.0;
                }
            }
        });
        #[cfg(not(feature = "installed"))]
        if let Some((before_ns, before_hits, before_generated)) = mip_before {
            let (after_ns, after_hits, after_generated) = painter.mip_diagnostic_counters();
            stage.mip_build_ms = after_ns.saturating_sub(before_ns) as f64 / 1_000_000.0;
            stage.mip_hits = after_hits.saturating_sub(before_hits);
            stage.mip_generated = after_generated.saturating_sub(before_generated);
            self.last_diagnostic_stages = stage;
        }
        #[cfg(test)]
        {
            stages.encode = encoding;
            stages.reset = reset_time;
            stages.replay = replay_time;
            stages.flush = flush_time;
            stages.raster = rasterizing;
            self.last_stages = stages;
        }
        if result.is_err() {
            self.tracker.forget();
        }
        result.map(|()| dirty)
    }

    pub(super) fn decorate(&mut self, canvas: &mut dyn Canvas) -> Option<Area> {
        self.overlay.pending = false;
        if !self.overlay.on {
            return self.overlay.show(canvas, &mut self.stamps, &[]);
        }
        let scale = self.params.map_or(1.0, |p| p.2);
        let mut outlines: Vec<(Area, Mark)> = crate::render::scroll::snapshot().into_iter().map(|r| (covering(r.viewport.scale_from_origin(scale)), Mark::Region)).collect();
        outlines.extend(self.shifted.iter().map(|a| (*a, Mark::Shift)));
        outlines.extend(self.live.iter().map(|l| (area(l.area), Mark::Layer)));
        outlines.extend(self.sprites.iter().filter(|s| !s.covered).map(|s| (covering(s.bounds()), Mark::Sprite)));
        self.overlay.show(canvas, &mut self.stamps, &outlines)
    }
}
