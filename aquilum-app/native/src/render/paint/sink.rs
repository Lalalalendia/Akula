//! Реализация трейта `PaintSink` для `Painter`.
use masonry::imaging::{
    BlurredRoundedRect, ClipRef, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink, StrokeRef,
};
use masonry::imaging::record::Glyph as SceneGlyph;
use masonry::kurbo::{Affine, Shape as _};
use masonry::peniko::{BlendMode, BrushRef};

use super::{convert, error::PaintError, mask, Painter, TOLERANCE};
mod path_guard;
mod shadow_guard;
mod opacity_guard;
mod image_guard;
mod brush_transform;
impl PaintSink for Painter {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        if self.error.is_some() { return; }
        if let Err(err) = self.guard_clip(&clip) { self.set_error_once(err); return; }
        if self.cull(&clip) { return; }
        if let ClipRef::Fill { transform, shape: GeometryRef::Rect(rect), fill_rule } = clip {
            let window = transform.transform_rect_bbox(rect);
            let [_, skew_y, skew_x, _, _, _] = transform.as_coeffs();
            let region = (skew_x == 0.0 && skew_y == 0.0 && self.regions.iter().any(|r| crate::render::scroll::close(*r, window))).then(|| crate::render::scroll::snap(window));
            self.ctx.set_fill_rule(fill_rule);
            match region {
                Some(snapped) => {
                    self.set_transform(Affine::IDENTITY);
                    self.ctx.push_clip_rect(&snapped);
                }
                None => {
                    self.set_transform(transform);
                    self.ctx.push_clip_rect(&rect);
                }
            }
        } else {
            let (transform, path, fill_rule) = convert::clip_to_path(clip, TOLERANCE);
            self.set_transform(transform);
            self.ctx.set_fill_rule(fill_rule);
            self.ctx.push_clip_path(&path);
        }
        self.clip_depth += 1;
    }
    fn pop_clip(&mut self) {
        if self.error.is_some() || self.uncull() {
            return;
        }
        if self.clip_depth == 0 {
            self.set_error_once(PaintError::Unbalanced("pop_clip без push_clip"));
            return;
        }
        self.ctx.pop_clip();
        self.clip_depth -= 1;
    }
    fn push_group(&mut self, group: GroupRef<'_>) {
        if let Err(err) = opacity_guard::validate(group.composite.alpha) { self.set_error_once(err); return; }
        if let Some(clip) = group.clip.as_ref() { if let Err(err) = self.guard_clip(clip) { self.set_error_once(err); return; } }
        if self.error.is_some() || (self.culled > 0 || group.composite.alpha <= 0.0) && self.cull_inside() || group.clip.as_ref().is_some_and(|clip| self.cull(clip)) {
            return;
        }
        let (clip_path, clip_transform) = match group.clip {
            None => (None, Affine::IDENTITY),
            Some(clip) => {
                let (transform, path, fill_rule) = convert::clip_to_path(clip, TOLERANCE);
                self.ctx.set_fill_rule(fill_rule);
                (Some(path), transform)
            }
        };
        self.set_transform(clip_transform);
        if let Err(err) = self.check_filter_budget(group.filters) { self.set_error_once(err); return; }
        let blend: Option<BlendMode> = Some(group.composite.blend);
        let opacity = Some(group.composite.alpha);
        let (width, height) = (self.width(), self.height());
        let mask = match group.mask {
            Some(mask) => {
                match mask::get_or_render_mask(
                    &mut self.mask_cache,
                    mask.mask.scene,
                    mask.mask.mode,
                    self.shift * mask.transform,
                    width,
                    height,
                    self.settings,
                ) {
                    Ok(m) => Some(m),
                    Err(err) => {
                        self.set_error_once(err);
                        None
                    }
                }
            }
            None => None,
        };
        self.push_filter_chain(clip_path.as_ref(), blend, opacity, mask, group.filters);
    }
    fn pop_group(&mut self) {
        if self.uncull() {
            return;
        }
        if self.error.is_some() {
            return;
        }
        if let Err(err) = self.pop_filter_chain() {
            self.set_error_once(err);
        }
    }
    fn fill(&mut self, draw: FillRef<'_>) {
        #[cfg(not(feature = "installed"))]
        {
            static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *ENABLED.get_or_init(|| {
                std::env::var("AKULA_EXPERIMENTAL_IMAGE_RESOLVE_PROOF")
                    .ok().as_deref() == Some("1")
            }) {
                if let BrushRef::Image(image) = &draw.brush {
                    let id = image.image.data.id();
                    let bounds = draw.transform.transform_rect_bbox(convert::shape_bounds(&draw.shape));
                    let offscreen = self.offscreen(bounds);
                    let sprite = crate::render::sprite::covered(id);
                    static SEEN: std::sync::OnceLock<
                        std::sync::Mutex<std::collections::HashSet<(u64, bool, Option<bool>)>>
                    > = std::sync::OnceLock::new();
                    if SEEN.get_or_init(Default::default)
                        .lock().unwrap_or_else(|e| e.into_inner())
                        .insert((id, offscreen, sprite))
                    {
                        eprintln!(
                            "AKULA_IMAGE_RASTER_FILL image_id={id} offscreen={offscreen} sprite={sprite:?} bounds={:.0},{:.0},{:.0},{:.0} frame={:.0},{:.0},{:.0},{:.0}",
                            bounds.x0, bounds.y0, bounds.x1, bounds.y1,
                            self.frame.x0, self.frame.y0, self.frame.x1, self.frame.y1
                        );
                    }
                }
            }
        }
        if let Err(err) = self.guard_geometry(&draw.shape, draw.transform) { self.set_error_once(err); return; }
        if let Err(err) = brush_transform::validate(draw.transform, draw.brush_transform) { self.set_error_once(err); return; }
        if let Err(err) = opacity_guard::validate_draw(&draw.brush, draw.composite.alpha) { self.set_error_once(err); return; }
        if let Err(err) = image_guard::validate(&draw.brush) { self.set_error_once(err); return; }
        if self.error.is_some() || convert::invisible(&draw.brush, draw.composite) || self.offscreen(draw.transform.transform_rect_bbox(convert::shape_bounds(&draw.shape))) {
            return;
        }
        if let BrushRef::Image(image) = &draw.brush
            && let Some(motion) = crate::render::live::motion(image.image.data.id())
        {
            return self.paint_live(draw.transform, &draw.shape, motion);
        }
        let sprite = match &draw.brush {
            BrushRef::Image(image) => crate::render::sprite::covered(image.image.data.id()).map(|covered| (covered, image.image.data.id())),
            _ => None,
        };
        let brush = match sprite {
            Some((false, _)) => return,
            Some((true, id)) => BrushRef::Solid(crate::render::sprite::color(id).unwrap_or(masonry::peniko::Color::TRANSPARENT)),
            None => draw.brush,
        };
        let (paint, paint_transform) = convert::brush_to_paint_trial(
            brush,
            draw.composite,
            &mut self.images,
            self.shift * draw.transform,
            draw.brush_transform.unwrap_or(Affine::IDENTITY),
        );
        self.set_transform(draw.transform);
        self.ctx.set_fill_rule(draw.fill_rule);
        self.ctx.set_paint_transform(paint_transform);
        self.ctx.set_blend_mode(draw.composite.blend);
        self.ctx.set_paint(paint);
        match draw.shape {
            GeometryRef::Rect(r) => self.ctx.fill_rect(&r),
            GeometryRef::RoundedRect(rr) => self.ctx.fill_path(&rr.to_path(TOLERANCE)),
            GeometryRef::Path(p) => self.ctx.fill_path(p),
            GeometryRef::OwnedPath(p) => self.ctx.fill_path(&p),
        }
    }
    fn stroke(&mut self, draw: StrokeRef<'_>) {
        if let Err(err) = self.guard_stroke(&draw.shape, draw.transform, &draw.stroke) { self.set_error_once(err); return; }
        if let Err(err) = brush_transform::validate(draw.transform, draw.brush_transform) { self.set_error_once(err); return; }
        if let Err(err) = opacity_guard::validate_draw(&draw.brush, draw.composite.alpha) { self.set_error_once(err); return; }
        if let Err(err) = image_guard::validate(&draw.brush) { self.set_error_once(err); return; }
        let w = draw.stroke.width.max(1.0) * draw.stroke.miter_limit.max(1.0);
        if self.error.is_some() || convert::invisible(&draw.brush, draw.composite) || self.offscreen(draw.transform.transform_rect_bbox(convert::shape_bounds(&draw.shape).inflate(w, w))) {
            return;
        }
        let paint = convert::brush_to_paint(draw.brush, draw.composite, &mut self.images);
        self.set_transform(draw.transform);
        self.ctx.set_stroke(draw.stroke.clone());
        self.ctx.set_paint_transform(draw.brush_transform.unwrap_or(Affine::IDENTITY));
        self.ctx.set_blend_mode(draw.composite.blend);
        self.ctx.set_paint(paint);
        match draw.shape {
            GeometryRef::Rect(r) => self.ctx.stroke_rect(&r),
            GeometryRef::RoundedRect(rr) => self.ctx.stroke_path(&rr.to_path(TOLERANCE)),
            GeometryRef::Path(p) => self.ctx.stroke_path(p),
            GeometryRef::OwnedPath(p) => self.ctx.stroke_path(&p),
        }
    }
    fn glyph_run(&mut self, draw: GlyphRunRef<'_>, glyphs: &mut dyn Iterator<Item = SceneGlyph>) {
        if self.error.is_some() { return; }
        if let Err(err) = opacity_guard::validate_draw(&draw.brush, draw.composite.alpha) { self.set_error_once(err); return; }
        if let Err(err) = image_guard::validate(&draw.brush) { self.set_error_once(err); return; }
        self.draw_glyph_run(draw, glyphs);
    }
    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        if self.error.is_some() {
            return;
        }
        if let Err(err) = shadow_guard::validate(&draw) { self.set_error_once(err); return; }
        self.set_transform(draw.transform);
        self.ctx.set_paint(draw.color.multiply_alpha(draw.composite.alpha));
        self.ctx.set_blend_mode(draw.composite.blend);
        self.ctx.fill_blurred_rounded_rect(&draw.rect, draw.radius as f32, draw.std_dev as f32, false);
    }
}
