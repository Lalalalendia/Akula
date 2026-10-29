use masonry::imaging::record::Glyph as SceneGlyph;
use masonry::imaging::{GeometryRef, GlyphRunRef};
use masonry::kurbo::{Affine, Rect};
use masonry::peniko::Style;
mod glyph_guard;

use super::Painter;
use super::convert;
use crate::render::live;

#[cfg(test)]
static TEST_GLYPH_ATLAS: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var_os("AQ_TEST_DISABLE_GLYPH_ATLAS").is_none());

impl Painter {
    pub(in crate::render) fn paint_live(&mut self, transform: Affine, shape: &GeometryRef<'_>, motion: live::Motion) {
        let Some(rounded) = live::shape(shape) else { return };
        let area = transform.transform_rect_bbox(rounded.rect()).intersect(self.frame).expand();
        if area.width() < 1.0 || area.height() < 1.0 {
            return;
        }
        let mut pixels = Vec::new();
        live::draw(transform, rounded, motion, area, self.seconds, &mut pixels);
        let metadata = vello_cpu::PixelMetadata::new(masonry::peniko::ImageAlphaType::AlphaPremultiplied, true);
        let pixmap = vello_cpu::Pixmap::from_parts(bytemuck::cast_slice::<u32, u8>(&pixels).to_vec(), area.width() as u16, area.height() as u16, metadata);
        let sampler = masonry::peniko::ImageSampler::default().with_quality(masonry::peniko::ImageQuality::Low);
        self.ctx.set_transform(self.shift * Affine::translate(area.origin().to_vec2()));
        self.ctx.set_paint_transform(Affine::IDENTITY);
        self.ctx.set_blend_mode(masonry::peniko::BlendMode::default());
        self.ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(std::sync::Arc::new(pixmap)), sampler });
        self.ctx.fill_rect(&Rect::new(0.0, 0.0, area.width(), area.height()));
    }

    pub(in crate::render) fn draw_glyph_run(&mut self, run: GlyphRunRef<'_>, glyphs: &mut dyn Iterator<Item = SceneGlyph>) {
        if let Err(err) = glyph_guard::validate_run(&run) { self.set_error_once(err); return; }
        let Some(first) = glyphs.next() else { return };
        if let Err(err) = glyph_guard::validate_glyph(run.transform, &first) { self.set_error_once(err); return; }
        let outside_first = self.offscreen(convert::line_bounds(
            first.y, run.font_size, run.glyph_transform, run.transform,
        ));
        #[cfg(test)]
        let outside_first = outside_first
            && (self.culled > 0
                || std::env::var_os("AQ_TEST_FULL_GLYPH_RUN_VISIBILITY").is_none());
        if outside_first {
            return;
        }
        let [a, b, c, d, _, _] = run.transform.as_coeffs();
        let reach = f64::from(run.font_size) * 1.5 * a.abs().max(b.abs()).max(c.abs()).max(d.abs());
        let area = self.frame.inflate(reach, reach);
        let filtered = self.filtered();
        let glyphs = match glyph_guard::collect(std::iter::once(first).chain(glyphs), run.transform, area, filtered) {
            Ok(value) => value,
            Err(err) => { self.set_error_once(err); return; }
        };
        if glyphs.is_empty() { return; }
        let paint = convert::brush_to_paint(run.brush, run.composite, &mut self.images);
        self.set_transform(run.transform);
        self.ctx.set_paint_transform(run.brush_transform.unwrap_or(Affine::IDENTITY));
        self.ctx.set_paint(paint);
        self.ctx.set_blend_mode(run.composite.blend);
        match run.style {
            Style::Fill(fill_rule) => self.ctx.set_fill_rule(*fill_rule),
            Style::Stroke(stroke) => self.ctx.set_stroke(stroke.clone()),
        }
        // Keep the same glyph atlas policy for incremental and full reference draws.
        #[cfg(test)]
        let atlas_cache = *TEST_GLYPH_ATLAS;
        #[cfg(not(test))]
        let atlas_cache = true;
        // Keep test-only hinting parity across cropped and full-frame draws.
        #[cfg(test)]
        let hint = run.hint && std::env::var_os("AQ_TEST_DISABLE_GLYPH_HINTING").is_none();
        #[cfg(not(test))]
        let hint = run.hint;
        let builder = self
            .ctx
            .glyph_run(&mut self.resources, run.font)
            .font_size(run.font_size)
            .atlas_cache(atlas_cache)
            .hint(hint)
            .normalized_coords(run.normalized_coords);
        let builder = match run.glyph_transform {
            Some(transform) => builder.glyph_transform(transform),
            None => builder,
        };
        let _ = match run.style {
            Style::Fill(_) => builder.fill_glyphs(glyphs.into_iter()),
            Style::Stroke(_) => builder.stroke_glyphs(glyphs.into_iter()),
        };
    }
}
