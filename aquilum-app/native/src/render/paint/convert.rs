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

// Counters are local to an opt-in developer renderer, with no logging in the
// per-image hot path. Installed builds do not compile these fields.
#[cfg(not(feature = "installed"))]
#[derive(Default)]
struct MipDiagnostics {
    requests: u64,
    final_hits: u64,
    generated: u64,
    generated_pixels: u64,
    evicted: u64,
    build_nanos: u128,
}

#[derive(Default)]
pub struct Images {
    sources: HashMap<u64, (ImageSource, u64, Option<Weak<()>>)>,
    /// Image ID + reduction level, restricted to a 32 MiB two-frame budget.
    mips: HashMap<(u64, u8), (Arc<Pixmap>, u64)>,
    frame: u64,
    #[cfg(not(feature = "installed"))]
    diag: MipDiagnostics,
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
        #[cfg(not(feature = "installed"))]
        {
            self.diag.requests += 1;
        }
        // A warmed final level is the common scrolling case. Avoid scanning
        // the budget and walking intermediate mip levels just to reuse it.
        if let Some((cached, last)) = self.mips.get_mut(&(id, level)) {
            *last = self.frame;
            #[cfg(not(feature = "installed"))]
            {
                self.diag.final_hits += 1;
            }
            return Some(ImageSource::Pixmap(Arc::clone(cached)));
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
            #[cfg(not(feature = "installed"))]
            let started = std::time::Instant::now();
            let next = Arc::new(mipmap::half(&previous)?);
            #[cfg(not(feature = "installed"))]
            {
                self.diag.generated += 1;
                self.diag.generated_pixels += u64::from(next.width()) * u64::from(next.height());
                self.diag.build_nanos += started.elapsed().as_nanos();
            }
            self.mips.insert((id, current), (Arc::clone(&next), frame));
            previous = next;
        }
        Some(ImageSource::Pixmap(previous))
    }

    pub fn next_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.sources.retain(|_, (_, used, owner)| frame - *used < IMAGE_FRAMES && owner.as_ref().is_none_or(|o| o.strong_count() > 0));
        #[cfg(not(feature = "installed"))]
        let previous_count = self.mips.len();
        self.mips.retain(|_, (_, last)| frame - *last < IMAGE_FRAMES);
        #[cfg(not(feature = "installed"))]
        {
            self.diag.evicted += (previous_count - self.mips.len()) as u64;
            // This diagnostic prints once per 128 real frame passes, not per
            // image paint, so it cannot dominate the measured frame p95.
            if frame % 128 == 0
                && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF").ok().as_deref() == Some("1")
            {
                eprintln!(
                    "AKULA_MIP_CACHE frames={frame} requests={} final_hits={} generated={} generated_pixels={} evicted={} generated_ms={:.3} resident_levels={}",
                    self.diag.requests, self.diag.final_hits, self.diag.generated,
                    self.diag.generated_pixels, self.diag.evicted,
                    self.diag.build_nanos as f64 / 1_000_000.0, self.mips.len(),
                );
            }
        }
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
        // A real GUI A/B is invalid without evidence that an image took this path.
        // Emit once only in explicit developer diagnostic mode.
        if !cfg!(feature = "installed")
            && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF").ok().as_deref() == Some("1")
        {
            static FIRST_APPLIED: std::sync::Once = std::sync::Once::new();
            FIRST_APPLIED.call_once(|| eprintln!("AKULA_MIP_APPLIED level={level}"));
        }
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
    fn warm_final_mip_shortcut_does_not_keep_unused_levels_alive() {
        let brush = test_brush(128, 128);
        let mut images = Images::default();
        let src = images.source(&brush.image);
        let id = brush.image.data.id();

        let first = images.mip_source(id, &src, 2).expect("initial build");
        assert_eq!(images.mips.len(), 2);
        images.next_frame();
        let reused = images.mip_source(id, &src, 2).expect("warm final level");
        let (ImageSource::Pixmap(first), ImageSource::Pixmap(reused)) = (first, reused)
        else { panic!("unexpected source kind") };
        assert!(Arc::ptr_eq(&first, &reused));
        #[cfg(not(feature = "installed"))]
        {
            assert_eq!(images.diag.requests, 2);
            assert_eq!(images.diag.final_hits, 1);
            assert_eq!(images.diag.generated, 2);
        }
        images.next_frame();
        assert!(!images.mips.contains_key(&(id, 1)), "unused intermediate may expire");
        assert!(images.mips.contains_key(&(id, 2)), "requested final stays resident");

        let lower = images.mip_source(id, &src, 1).expect("lower level rebuilt");
        let ImageSource::Pixmap(lower) = lower else { panic!("unexpected source") };
        assert_eq!((lower.width(), lower.height()), (64, 64));
    }

    #[test]
    fn multiple_damage_regions_share_one_image_cache_epoch() {
        let mut painter = super::super::Painter::new(128, 128, super::super::settings(0));
        let brush = test_brush(128, 128);

        painter.begin(0, 0, 128, 128);
        let (first, _) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut painter.images,
            Affine::scale(1.0 / 8.0), Affine::IDENTITY, true,
        );
        let Brush::Image(first) = first else { panic!("expected mip image") };
        let ImageSource::Pixmap(first) = first.image else { panic!("expected pixmap") };
        assert_eq!(painter.images.mips.len(), 2);
        let epoch = painter.images.frame;

        // Same visual frame, second dirty rectangle. Both Vello context and
        // clip reset, but the very same mip source must remain reusable.
        painter.begin_region(16, 0, 64, 128);
        assert_eq!(painter.images.frame, epoch);
        let (second, _) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut painter.images,
            Affine::scale(1.0 / 8.0), Affine::IDENTITY, true,
        );
        let Brush::Image(second) = second else { panic!("expected mip image") };
        let ImageSource::Pixmap(second) = second.image else { panic!("expected pixmap") };
        assert!(Arc::ptr_eq(&first, &second), "do not rebuild mip per dirty region");

        // New frame advances the epoch, but the latest mip may still be used.
        painter.begin(0, 0, 128, 128);
        assert_eq!(painter.images.frame, epoch + 1);
        assert_eq!(painter.images.mips.len(), 2);
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


// This exercises the actual Akula image-brush converter and Vello CPU rasterizer
// on an in-repository PNG. The reference is a direct full-footprint area sum,
// independent of mipmap::half and of Vello's sampling implementation.
#[cfg(test)]
mod image_scene_tests {
    use super::*;
    use masonry::peniko::{ImageAlphaType, ImageQuality};
    use vello_cpu::color::palette::css::WHITE;
    use vello_cpu::{RasterizerSettings, RenderContext, RenderSettings, Resources, TargetInit};

    const SIDE: usize = 128;

    #[derive(Clone, Copy, Debug)]
    enum Scene {
        Photo,
        AlphaCrop,
    }

    fn fixture(scene: Scene) -> Vec<u8> {
        // A real repository asset, not a repeated synthetic stripe texture.
        let photo = image::load_from_memory(include_bytes!("../../../assets/city-cover.png"))
            .expect("pinned local city cover PNG")
            .to_rgba8();
        let mut bytes = image::imageops::resize(
            &photo, SIDE as u32, SIDE as u32, image::imageops::FilterType::Lanczos3,
        )
        .into_raw();
        if matches!(scene, Scene::AlphaCrop) {
            // Use straight-alpha input through the same decoding/registration
            // semantics as Markdown images. Fully transparent texels retain
            // their RGB so a non-premultiplied filter would produce halos.
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let pixel = (y * SIDE + x) * 4;
                    bytes[pixel + 3] = [0, 64, 128, 192, 255][(x / 7 + y / 11) % 5];
                }
            }
        }
        bytes
    }

    fn shape(scene: Scene) -> Rect {
        match scene {
            Scene::Photo => Rect::new(0.0, 0.0, 128.0, 128.0),
            Scene::AlphaCrop => Rect::new(16.0, 16.0, 112.0, 112.0),
        }
    }

    fn draw(data: &[u8], ratio: u16, scene: Scene, enabled: bool, workers: u16) -> (Pixmap, usize) {
        let brush = crate::render::pixels::from_rgba(
            data.to_vec(), SIDE as u32, SIDE as u32, ImageAlphaType::Alpha,
        )
        .expect("valid source fixture")
        .with_quality(ImageQuality::High);

        let transform = Affine::scale(1.0 / f64::from(ratio));
        let mut images = Images::default();
        let (paint, brush_transform) = brush_to_paint_candidate(
            BrushRef::Image(brush.as_ref()), Composite::default(),
            &mut images, transform, Affine::IDENTITY, enabled,
        );
        let allocated = images.mips.len();
        if enabled {
            assert!(allocated > 0, "a quality test must exercise actual mip generation");
        } else {
            assert_eq!(allocated, 0, "the baseline may not allocate candidate mipmaps");
        }

        let size = (SIDE as u16) / ratio;
        let mut ctx = RenderContext::new_with(
            size, size, RenderSettings { num_threads: workers, ..RenderSettings::default() },
        );
        ctx.set_transform(transform);
        ctx.set_paint_transform(brush_transform);
        ctx.set_paint(paint);
        ctx.fill_rect(&shape(scene));
        ctx.flush();
        let mut out = Pixmap::new(size, size);
        ctx.render_with(
            &mut out, &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..RasterizerSettings::default()
            },
        );
        (out, allocated)
    }

    // Straight RGBA -> premultiplied area average -> source-over white.
    // Unlike the candidate, this reference sums *every* source texel in the
    // destination's footprint in one pass, without iterative halving.
    fn area_reference(data: &[u8], x: usize, y: usize, ratio: usize) -> [u8; 3] {
        let mut rgba = [0u64; 4];
        for iy in (y * ratio)..((y + 1) * ratio) {
            for ix in (x * ratio)..((x + 1) * ratio) {
                let index = (iy * SIDE + ix) * 4;
                let a = u64::from(data[index + 3]);
                for channel in 0..3 {
                    // Premultiplication is deliberately independent of Vello.
                    rgba[channel] += (u64::from(data[index + channel]) * a + 127) / 255;
                }
                rgba[3] += a;
            }
        }
        let count = (ratio * ratio) as u64;
        let a = (rgba[3] + count / 2) / count;
        let mut expected = [255u8; 3];
        for channel in 0..3 {
            let value = (rgba[channel] + count / 2) / count;
            expected[channel] = value.saturating_add(255 - a).min(255) as u8;
        }
        expected
    }

    fn error_against_area(
        data: &[u8], frame: &Pixmap, ratio: u16, scene: Scene,
    ) -> (f64, u8, usize) {
        let side = SIDE / usize::from(ratio);
        let mut total = 0u64;
        let mut worst = 0u8;
        let mut compared = 0usize;
        for y in 0..side {
            for x in 0..side {
                let score = match scene {
                    Scene::Photo => x > 0 && y > 0 && x + 1 < side && y + 1 < side,
                    Scene::AlphaCrop => {
                        let left = 16 / usize::from(ratio);
                        let right = 112 / usize::from(ratio);
                        x > left && y > left && x + 1 < right && y + 1 < right
                    }
                };
                if !score { continue; }
                let expect = area_reference(data, x, y, usize::from(ratio));
                let px = frame.data()[y * side + x];
                for (actual, wanted) in [px.r, px.g, px.b].into_iter().zip(expect) {
                    let delta = actual.abs_diff(wanted);
                    total += u64::from(delta);
                    worst = worst.max(delta);
                    compared += 1;
                }
            }
        }
        assert!(compared > 0, "need interior pixels for an independent reference");
        (total as f64 / compared as f64, worst, compared)
    }

    #[test]
    fn real_png_and_straight_alpha_crop_match_independent_area_reference() {
        let mut checked = 0usize;
        for workers in [0, 2] {
            for scene in [Scene::Photo, Scene::AlphaCrop] {
                let data = fixture(scene);
                for ratio in [4u16, 8, 16] {
                    let (before, _) = draw(&data, ratio, scene, false, workers);
                    let (after, mips) = draw(&data, ratio, scene, true, workers);
                    let (base_mae, base_max, pixels) =
                        error_against_area(&data, &before, ratio, scene);
                    let (trial_mae, trial_max, _) =
                        error_against_area(&data, &after, ratio, scene);

                    if matches!(scene, Scene::AlphaCrop) {
                        // Cropping must not move the brush or paint into the
                        // untouched top-left corner of the white canvas.
                        let corner = after.data()[0];
                        assert_eq!([corner.r, corner.g, corner.b, corner.a], [255, 255, 255, 255]);
                    }
                    println!(
                        "AKULA_MIP_REALPNG scene={scene:?} ratio={ratio} workers={workers} \
                         baseline_mae={base_mae:.3} candidate_mae={trial_mae:.3} \
                         baseline_max={base_max} candidate_max={trial_max} \
                         compared_channels={pixels} allocated_mips={mips}"
                    );
                    // Fail on a material quality loss rather than asking only
                    // whether the specialized stripe fixture became green.
                    // Tighten once we inspect the independent A/B distributions.
                    assert!(
                        trial_mae <= base_mae + 4.0 && trial_mae <= 25.0,
                        "candidate outside conservative real-image quality guard \
                         scene={scene:?} ratio={ratio} workers={workers} \
                         baseline={base_mae:.3} candidate={trial_mae:.3}"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 12);
    }
}
