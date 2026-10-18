//! Преобразования типов между Masonry/imaging и vello_cpu.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use masonry::imaging::{ClipRef, Composite, Filter, GeometryRef};
use masonry::kurbo::{Affine, BezPath, Rect, Shape as _, StrokeOpts, stroke};
use masonry::peniko::{Brush, BrushRef, Extend, Fill, ImageData, ImageQuality};
use vello_cpu::filter_effects::{EdgeMode, Filter as VelloFilter, FilterGraph, FilterPrimitive};
use vello_cpu::{Image, ImageSource, PaintType, Pixmap};

use super::mipmap;

const IMAGE_FRAMES: u64 = 2;

#[derive(Default)]
pub struct Images {
    sources: HashMap<u64, (ImageSource, u64, Option<Weak<()>>)>,
    /// Image ID + reduction level, restricted to a 32 MiB two-frame budget.
    mips: HashMap<(u64, u8), (Arc<Pixmap>, u64)>,
    frame: u64,
}

impl Images {
    fn source(&mut self, data: &ImageData) -> ImageSource {
        let frame = self.frame;
        let (source, used, _) = self.sources.entry(data.data.id()).or_insert_with(|| {
            if let Some((pixmap, owner)) = crate::render::surface::find(data.data.id()) {
                return (ImageSource::Pixmap(pixmap), frame, Some(owner));
            }
            let source = match crate::render::pixels::lookup(data.data.id()) {
                Some(pixmap) => ImageSource::Pixmap(pixmap),
                None if data.data.data().len() != data.width as usize * data.height as usize * 4 => ImageSource::Pixmap(Arc::new(vello_cpu::Pixmap::new(1, 1))),
                None => ImageSource::from_peniko_image_data(data),
            };
            (source, frame, None)
        });
        *used = frame;
        source.clone()
    }

    /// Reuse a premultiplied mipmap for the same image ID and level.
    fn mip_source(&mut self, id: u64, original: &ImageSource, level: u8) -> Option<ImageSource> {
        self.mip_source_budget(id, original, level, mipmap::MAX_EXTRA_BYTES)
    }

    /// Reserve every missing mip level BEFORE allocating anything. A request
    /// denied by the budget must not populate partial, unusable cache levels.
    fn mip_source_budget(&mut self, id: u64, original: &ImageSource, level: u8, budget: usize) -> Option<ImageSource> {
        let ImageSource::Pixmap(base) = original else { return None };
        if level == 0 || level > mipmap::MAX_LEVELS
            || base.width() == 0 || base.height() == 0
            || usize::from(base.width()) * usize::from(base.height()) > mipmap::MAX_PIXELS
        {
            return None;
        }
        let used_bytes: usize = self.mips.values()
            .map(|(mip, _)| usize::from(mip.width()) * usize::from(mip.height()) * 4)
            .sum();
        let (mut width, mut height) = (usize::from(base.width()), usize::from(base.height()));
        let mut new_bytes = 0usize;
        for current in 1..=level {
            if width == 1 && height == 1 { return None; }
            width = width.div_ceil(2);
            height = height.div_ceil(2);
            if !self.mips.contains_key(&(id, current)) {
                new_bytes = new_bytes.saturating_add(width * height * 4);
            }
        }
        if used_bytes.saturating_add(new_bytes) > budget { return None; }

        let mut previous = Arc::clone(base);
        let frame = self.frame;
        for current in 1..=level {
            if let Some((cached, last)) = self.mips.get_mut(&(id, current)) {
                *last = frame;
                previous = Arc::clone(cached);
                continue;
            }
            let next = Arc::new(mipmap::half(&previous)?);
            self.mips.insert((id, current), (Arc::clone(&next), frame));
            previous = next;
        }
        Some(ImageSource::Pixmap(previous))
    }

    pub fn next_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.sources.retain(|_, (_, used, owner)| frame - *used < IMAGE_FRAMES && owner.as_ref().is_none_or(|o| o.strong_count() > 0));
        self.mips.retain(|_, (_, last)| frame - *last < IMAGE_FRAMES);
    }
}

pub fn brush_to_paint(brush: BrushRef<'_>, composite: Composite, images: &mut Images) -> PaintType {
    match brush.to_owned().multiply_alpha(composite.alpha) {
        Brush::Solid(c) => Brush::Solid(c),
        Brush::Gradient(g) => Brush::Gradient(g),
        Brush::Image(image) => Brush::Image(Image { image: images.source(&image.image), sampler: image.sampler }),
    }
}


/// Experimental Akula-owned image minification. Never intercepts glifo's
/// glyph atlas or touches non-image brushes. Enabled only on uninstalled
/// builds with AKULA_EXPERIMENTAL_IMAGE_MIP=1.
pub fn brush_to_paint_trial(
    brush: BrushRef<'_>,
    composite: Composite,
    images: &mut Images,
    effective_geometry: Affine,
    brush_transform: Affine,
) -> (PaintType, Affine) {
    let enabled = !cfg!(feature = "installed")
        && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP").ok().as_deref() == Some("1");
    brush_to_paint_candidate(
        brush, composite, images, effective_geometry, brush_transform, enabled,
    )
}

// Testable decision path without changing process environment or allowing the
// experimental setting to leak between concurrently running Rust tests.
fn brush_to_paint_candidate(
    brush: BrushRef<'_>,
    composite: Composite,
    images: &mut Images,
    effective_geometry: Affine,
    brush_transform: Affine,
    enabled: bool,
) -> (PaintType, Affine) {
    let original_id = if let BrushRef::Image(image) = &brush {
        Some(image.image.data.id())
    } else {
        None
    };
    let mut paint = brush_to_paint(brush, composite, images);
    if !enabled {
        return (paint, brush_transform);
    }
    let Some(id) = original_id else { return (paint, brush_transform) };
    let Brush::Image(image) = &mut paint else { return (paint, brush_transform) };
    if !matches!(image.sampler.quality, ImageQuality::Medium | ImageQuality::High)
        || image.sampler.x_extend != Extend::Pad
        || image.sampler.y_extend != Extend::Pad
    {
        return (paint, brush_transform);
    }
    let level = mipmap::level_for_transform(effective_geometry * brush_transform);
    if level == 0 { return (paint, brush_transform); }
    if let Some(reduced) = images.mip_source(id, &image.image, level) {
        image.image = reduced;
        // Compensate the reduced image inside the existing brush transform.
        // Geometry, clipping, draw alpha and alignment stay unchanged.
        return (paint, brush_transform * Affine::scale(f64::from(1_u16 << level)));
    }
    (paint, brush_transform)
}

pub fn geometry_to_path(geom: GeometryRef<'_>, tolerance: f64) -> BezPath {
    match geom {
        GeometryRef::Rect(r) => r.to_path(tolerance),
        GeometryRef::RoundedRect(rr) => rr.to_path(tolerance),
        GeometryRef::Path(p) => p.clone(),
        GeometryRef::OwnedPath(p) => p,
    }
}

pub fn clip_to_path(clip: ClipRef<'_>, tolerance: f64) -> (Affine, BezPath, Fill) {
    match clip {
        ClipRef::Fill { transform, shape, fill_rule } => {
            (transform, geometry_to_path(shape, tolerance), fill_rule)
        }
        ClipRef::Stroke { transform, shape, stroke: style } => {
            let path = geometry_to_path(shape, tolerance);
            let outline = stroke(path.iter(), style, &StrokeOpts::default(), tolerance);
            (transform, outline, Fill::NonZero)
        }
    }
}

pub fn filters_to_vello(filters: &[Filter]) -> Option<VelloFilter> {
    let mut graph = FilterGraph::new();
    let mut last = None;
    for f in filters {
        let primitive = match *f {
            Filter::Flood { color } => FilterPrimitive::Flood { color },
            Filter::Blur { std_deviation_x, std_deviation_y } => FilterPrimitive::GaussianBlur {
                std_deviation: std_deviation_x.max(std_deviation_y),
                edge_mode: EdgeMode::None,
            },
            Filter::DropShadow { dx, dy, std_deviation_x, std_deviation_y, color } => {
                FilterPrimitive::DropShadow {
                    dx,
                    dy,
                    std_deviation: std_deviation_x.max(std_deviation_y),
                    color,
                    edge_mode: EdgeMode::None,
                }
            }
            Filter::Offset { dx, dy } => FilterPrimitive::Offset { dx, dy },
        };
        last = Some(graph.add(primitive, None));
    }
    let out = last?;
    graph.set_output(out);
    Some(VelloFilter { graph: Arc::new(graph) })
}

pub fn invisible(brush: &BrushRef<'_>, composite: Composite) -> bool {
    composite.alpha <= 0.0 || composite.blend == masonry::peniko::BlendMode::default() && matches!(brush, BrushRef::Solid(color) if color.components[3] <= 0.0)
}

pub fn shape_bounds(shape: &GeometryRef<'_>) -> Rect {
    match shape {
        GeometryRef::Rect(r) => *r,
        GeometryRef::RoundedRect(rr) => rr.rect(),
        GeometryRef::Path(p) => p.bounding_box(),
        GeometryRef::OwnedPath(p) => p.bounding_box(),
    }
}

pub fn line_bounds(y: f32, font_size: f32, glyph_transform: Option<Affine>, transform: Affine) -> Rect {
    let size = f64::from(font_size);
    let local = Rect::new(-1e7, f64::from(y) - size * 1.25, 1e7, f64::from(y) + size * 0.5);
    let local = glyph_transform.map_or(local, |g| g.transform_rect_bbox(local).union(local));
    transform.transform_rect_bbox(local)
}


#[cfg(test)]
mod mipmap_integration_tests {
    use super::*;
    use masonry::peniko::{ImageAlphaType, ImageBrush};
    use vello_cpu::PixelMetadata;

    fn test_brush(width: u16, height: u16) -> ImageBrush {
        let raw = vec![120, 60, 30, 255].repeat(usize::from(width) * usize::from(height));
        crate::render::pixels::brush(Pixmap::from_parts(
            raw, width, height,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, false),
        )).with_quality(ImageQuality::High)
    }

    #[test]
    fn paint_selection_is_opt_in_and_keeps_image_geometry() {
        let brush = test_brush(128, 128);
        let mut images = Images::default();
        let geometry = Affine::scale(1.0 / 8.0);
        let (baseline, untouched) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            geometry, Affine::IDENTITY, false,
        );
        assert_eq!(untouched, Affine::IDENTITY);
        let Brush::Image(original) = baseline else { panic!("expected original image") };
        let ImageSource::Pixmap(original) = original.image else { panic!("expected pixmap") };
        assert_eq!((original.width(), original.height()), (128, 128));
        assert!(images.mips.is_empty());

        let (enabled, image_transform) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            geometry, Affine::IDENTITY, true,
        );
        assert_eq!(image_transform, Affine::scale(4.0));
        let Brush::Image(candidate) = enabled else { panic!("expected candidate image") };
        let ImageSource::Pixmap(candidate) = candidate.image else { panic!("expected pixmap") };
        assert_eq!((candidate.width(), candidate.height()), (32, 32));
        assert!(!candidate.may_have_transparency());
        assert_eq!(images.mips.len(), 2);

        let (again, _) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            geometry, Affine::IDENTITY, true,
        );
        let Brush::Image(again) = again else { panic!("expected cached image") };
        let ImageSource::Pixmap(again) = again.image else { panic!("expected pixmap") };
        assert!(Arc::ptr_eq(&candidate, &again), "same id/level reuses the mip");
        images.next_frame();
        assert_eq!(images.mips.len(), 2);
        images.next_frame();
        assert!(images.mips.is_empty(), "mips expire after two unused frames");
    }

    #[test]
    fn odd_dimensions_are_prefiltered_with_original_draw_geometry() {
        let brush = test_brush(127, 129);
        let mut images = Images::default();
        let (paint, transform) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            Affine::scale(1.0 / 8.0), Affine::IDENTITY, true,
        );
        assert_eq!(transform, Affine::scale(4.0));
        let Brush::Image(image) = paint else { panic!("expected image") };
        let ImageSource::Pixmap(image) = image.image else { panic!("expected pixmap") };
        assert_eq!((image.width(), image.height()), (32, 33));
        assert_eq!(images.mips.len(), 2);
    }

    #[test]
    fn denied_mip_request_does_not_allocate_intermediate_levels() {
        let brush = test_brush(128, 128);
        let mut images = Images::default();
        let source = images.source(&brush.image);
        let id = brush.image.data.id();
        let needed = 64 * 64 * 4 + 32 * 32 * 4;
        assert!(images.mip_source_budget(id, &source, 2, needed - 1).is_none());
        assert!(images.mips.is_empty(), "no partial allocation on failed reservation");
        assert!(images.mip_source_budget(id, &source, 2, needed).is_some());
        assert_eq!(images.mips.len(), 2);
    }

    #[test]
    fn image_low_quality_retains_legacy_sampling() {
        let brush = test_brush(127, 129).with_quality(ImageQuality::Low);
        let mut images = Images::default();
        let (paint, transform) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            Affine::scale(1.0 / 8.0), Affine::IDENTITY, true,
        );
        assert_eq!(transform, Affine::IDENTITY);
        let Brush::Image(image) = paint else { panic!("expected image") };
        let ImageSource::Pixmap(image) = image.image else { panic!("expected pixmap") };
        assert_eq!((image.width(), image.height()), (127, 129));
        assert!(images.mips.is_empty());
    }
}
