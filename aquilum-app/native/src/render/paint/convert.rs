//! Преобразования типов между Masonry/imaging и vello_cpu.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use masonry::imaging::{ClipRef, Composite, Filter, GeometryRef};
use masonry::kurbo::{Affine, BezPath, Rect, Shape as _, StrokeOpts, stroke};
use masonry::peniko::{Brush, BrushRef, Fill, ImageData};
#[cfg(any(test, not(feature = "installed")))]
use masonry::peniko::{Extend, ImageQuality};
use vello_cpu::filter_effects::{EdgeMode, Filter as VelloFilter, FilterPrimitive};
use vello_cpu::{Image, ImageSource, PaintType};
#[cfg(any(test, not(feature = "installed")))]
use vello_cpu::Pixmap;

#[cfg(any(test, not(feature = "installed")))]
use super::mipmap;

const IMAGE_FRAMES: u64 = 2;
#[cfg(any(test, not(feature = "installed")))]
const EXPERIMENTAL_MIP_LINGER_FRAMES: u64 = 512;

// Experimental, per-process fixed cache policy. The installed binary cannot
// activate it. Explicit opt-in keeps the old 2-frame policy intact otherwise.
#[cfg(any(test, not(feature = "installed")))]
fn mip_linger_enabled() -> bool {
    static LINGER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LINGER.get_or_init(|| {
        !cfg!(feature = "installed")
            && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP").ok().as_deref() == Some("1")
            && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP_LINGER").ok().as_deref() == Some("1")
    })
}

// Developer-only per-image prefilter decision provenance. Trace ephemeral
// image IDs, never filenames/paths/pixel bytes. Once per (ID, reason).
#[cfg(not(feature = "installed"))]
fn trace_mip_decision(
    id: u64, reason: &'static str, level: u8, source: &ImageSource,
    geometry: Affine, brush: Affine,
) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ENABLED.get_or_init(|| {
        std::env::var("AKULA_EXPERIMENTAL_IMAGE_RESOLVE_PROOF").ok().as_deref() == Some("1")
    }) {
        return;
    }
    static SEEN: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<(u64, &'static str)>>
    > = std::sync::OnceLock::new();
    if SEEN.get_or_init(Default::default)
        .lock().unwrap_or_else(|e| e.into_inner())
        .insert((id, reason))
    {
        let g = geometry.as_coeffs();
        let b = brush.as_coeffs();
        eprintln!(
            "AKULA_IMAGE_MIP_DECISION image_id={id} reason={reason} level={level} pixmap={} geom={g:?} brush={b:?}",
            matches!(source, ImageSource::Pixmap(_))
        );
    }
}

// Developer-only selective L2 experiment. This is never an installed feature.
#[cfg(any(test, not(feature = "installed")))]
fn mip_direct_l2_enabled() -> bool {
    static DIRECT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DIRECT.get_or_init(|| {
        !cfg!(feature = "installed")
            && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP").ok().as_deref() == Some("1")
            && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP_DIRECT_L2").ok().as_deref() == Some("1")
    })
}

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
    distinct_requested_ids: std::collections::HashSet<u64>,
    // Only hashed on the first request for each runtime ID; three
    // complete scanlines are sufficient to distinguish the deterministic
    // benchmark's seeded PNGs. This is NOT a full-content file hash.
    distinct_sampled_pixel_fingerprints: std::collections::HashSet<blake3::Hash>,
}

#[derive(Default)]
pub struct Images {
    sources: HashMap<u64, (ImageSource, u64, Option<Weak<()>>)>,
    /// Image ID + level, strictly bounded to 32 MiB. Only developer builds
    /// compile/use this cache; installed production has no mip-cache field.
    #[cfg(any(test, not(feature = "installed")))]
    mips: HashMap<(u64, u8), (Arc<Pixmap>, u64)>,
    frame: u64,
    #[cfg(not(feature = "installed"))]
    diag: MipDiagnostics,
}

impl Images {
    #[cfg(not(feature = "installed"))]
    pub(super) fn diagnostic_counters(&self) -> (u128, u64, u64) {
        (self.diag.build_nanos, self.diag.final_hits, self.diag.generated)
    }

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
    #[cfg(any(test, not(feature = "installed")))]
    fn mip_source(&mut self, id: u64, original: &ImageSource, level: u8) -> Option<ImageSource> {
        self.mip_source_budget(id, original, level, mipmap::MAX_EXTRA_BYTES)
    }

    /// Reserve every missing mip level BEFORE allocating anything. A request
    /// denied by the budget must not populate partial, unusable cache levels.
    #[cfg(any(test, not(feature = "installed")))]
    fn mip_source_budget(&mut self, id: u64, original: &ImageSource, level: u8, budget: usize) -> Option<ImageSource> {
        self.mip_source_with_policy(id, original, level, budget, mip_linger_enabled())
    }

    #[cfg(any(test, not(feature = "installed")))]
    fn mip_source_with_policy(
        &mut self, id: u64, original: &ImageSource, level: u8,
        budget: usize, reclaim_old: bool,
    ) -> Option<ImageSource> {
        self.mip_source_with_strategy(
            id, original, level, budget, reclaim_old, mip_direct_l2_enabled(),
        )
    }

    // Explicit strategy parameter prevents env-dependence in deterministic tests.
    #[cfg(any(test, not(feature = "installed")))]
    fn mip_source_with_strategy(
        &mut self, id: u64, original: &ImageSource, level: u8,
        budget: usize, reclaim_old: bool, direct_l2: bool,
    ) -> Option<ImageSource> {
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
            // Count distinct source images actually used by the sampler,
            // not just filenames enumerated in a benchmark note. Enabled
            // only for multisource diagnostic runs.
            static DISTINCT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *DISTINCT.get_or_init(|| std::env::var("AKULA_EXPERIMENTAL_IMAGE_DISTINCT_PROOF").ok().as_deref() == Some("1"))
                && self.diag.distinct_requested_ids.insert(id)
            {
                // Do not equate fresh runtime IDs with distinct image bytes.
                // Fingerprint the first/middle/last *decoded* RGBA scanline
                // once per ID. The seeded fixture has unique first-row RGBA
                // values for every PNG, so this is an independent contents
                // witness. Hashing the whole 4.3MiB source during a measured
                // frame would substantially perturb CPU p95.
                let row_bytes = usize::from(base.width()) * 4;
                let height = usize::from(base.height());
                let raw = base.data_as_u8_slice();
                if row_bytes > 0 && height > 0 && raw.len() >= row_bytes * height {
                    let mut fingerprint = blake3::Hasher::new();
                    fingerprint.update(&base.width().to_le_bytes());
                    fingerprint.update(&base.height().to_le_bytes());
                    for row in [0, height / 2, height - 1] {
                        let offset = row * row_bytes;
                        fingerprint.update(&raw[offset..offset + row_bytes]);
                    }
                    let digest = fingerprint.finalize();
                    if std::env::var("AKULA_EXPERIMENTAL_IMAGE_RESOLVE_PROOF")
                        .ok().as_deref() == Some("1")
                    {
                        // This proves requested pixel content for this blob
                        // ID. A separate converter 'used' result proves that
                        // a mip was successfully returned for rasterization.
                        eprintln!("AKULA_IMAGE_MIP_CONTENT image_id={id} fingerprint={digest}");
                    }
                    self.diag.distinct_sampled_pixel_fingerprints.insert(digest);
                }
            }
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
        // The optional cold direct L2 path reserves no unused L1.
        // If L1 already exists, retain the original incremental algorithm.
        let direct_cold_l2 = direct_l2 && level == 2
            && !self.mips.contains_key(&(id, 1));
        let used_bytes: usize = self.mips.values()
            .map(|(mip, _)| usize::from(mip.width()) * usize::from(mip.height()) * 4)
            .sum();
        let (mut width, mut height) = (usize::from(base.width()), usize::from(base.height()));
        let mut new_bytes = 0usize;
        for current in 1..=level {
            if width == 1 && height == 1 { return None; }
            width = width.div_ceil(2);
            height = height.div_ceil(2);
            if !(direct_cold_l2 && current == 1)
                && !self.mips.contains_key(&(id, current))
            {
                new_bytes = new_bytes.saturating_add(width * height * 4);
            }
        }
        // In long-lived mode, evict only LRU levels belonging to OTHER
        // sources. Never remove an intermediate level that was counted as
        // present when reserving this request's memory. Decide all removals
        // before mutating the cache: failed reservations stay atomic.
        let mut eviction = Vec::new();
        if used_bytes.saturating_add(new_bytes) > budget {
            if !reclaim_old || new_bytes > budget { return None; }
            let mut lru: Vec<_> = self.mips.iter()
                .filter(|((image_id, _), _)| *image_id != id)
                .map(|(&key, (pixmap, last))| {
                    (key, *last, usize::from(pixmap.width()) * usize::from(pixmap.height()) * 4)
                })
                .collect();
            lru.sort_unstable_by_key(|&(_, last, _)| last);
            let mut freed_bytes = 0usize;
            for (key, _, bytes) in lru {
                if used_bytes.saturating_sub(freed_bytes).saturating_add(new_bytes) <= budget {
                    break;
                }
                freed_bytes = freed_bytes.saturating_add(bytes);
                eviction.push(key);
            }
            if used_bytes.saturating_sub(freed_bytes).saturating_add(new_bytes) > budget {
                return None;
            }
        }
        for key in eviction {
            if self.mips.remove(&key).is_some() {
                #[cfg(not(feature = "installed"))]
                {
                    self.diag.evicted += 1;
                }
            }
        }

        // The L2-only reduction is byte-identical to two rounded box passes,
        // but writes only the final level. Missing L1 is built lazily on zoom.
        if direct_cold_l2 {
            #[cfg(not(feature = "installed"))]
            let started = std::time::Instant::now();
            let next = Arc::new(mipmap::direct_l2_packed(base)?);
            #[cfg(not(feature = "installed"))]
            {
                self.diag.generated += 1;
                self.diag.generated_pixels += u64::from(next.width()) * u64::from(next.height());
                self.diag.build_nanos += started.elapsed().as_nanos();
            }
            self.mips.insert((id, 2), (Arc::clone(&next), self.frame));
            return Some(ImageSource::Pixmap(next));
        }
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
        #[cfg(all(feature = "installed", not(test)))]
        {
            self.frame += 1;
            let frame = self.frame;
            self.sources.retain(|_, (_, used, owner)| {
                frame - *used < IMAGE_FRAMES
                    && owner.as_ref().is_none_or(|o| o.strong_count() > 0)
            });
        }
        #[cfg(any(test, not(feature = "installed")))]
        {
            let mip_frames = if mip_linger_enabled() {
                EXPERIMENTAL_MIP_LINGER_FRAMES
            } else {
                IMAGE_FRAMES
            };
            self.next_frame_with_mip_lifetime(mip_frames);
        }
    }

    #[cfg(any(test, not(feature = "installed")))]
    fn next_frame_with_mip_lifetime(&mut self, mip_frames: u64) {
        self.frame += 1;
        let frame = self.frame;
        self.sources.retain(|_, (_, used, owner)| frame - *used < IMAGE_FRAMES && owner.as_ref().is_none_or(|o| o.strong_count() > 0));
        #[cfg(not(feature = "installed"))]
        let previous_count = self.mips.len();
        self.mips.retain(|_, (_, last)| frame - *last < mip_frames);
        #[cfg(not(feature = "installed"))]
        {
            self.diag.evicted += (previous_count - self.mips.len()) as u64;
            // This diagnostic prints once per 128 real frame passes, not per
            // image paint, so it cannot dominate the measured frame p95.
            // Keep normal diagnostics sparse; in the multisource stress
            // experiment report every 32 frames so the last sample does not
            // undercount sources visited during a six-second fling.
            let multisource = std::env::var("AKULA_EXPERIMENTAL_IMAGE_DISTINCT_PROOF")
                .ok().as_deref() == Some("1");
            if (frame % 128 == 0 || (multisource && frame % 32 == 0))
                && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF").ok().as_deref() == Some("1")
            {
                eprintln!(
                    "AKULA_MIP_CACHE frames={frame} requests={} final_hits={} generated={} generated_pixels={} evicted={} generated_ms={:.3} resident_levels={} distinct_requested_ids={} distinct_sampled_pixel_fingerprints={}",
                    self.diag.requests, self.diag.final_hits, self.diag.generated,
                    self.diag.generated_pixels, self.diag.evicted,
                    self.diag.build_nanos as f64 / 1_000_000.0, self.mips.len(),
                    self.diag.distinct_requested_ids.len(),
                    self.diag.distinct_sampled_pixel_fingerprints.len(),
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


/// Installed Akula never even examines the experimental image flags.
/// This is the original paint path, with no mipmap cache allocation or
/// developer-only environment lookups in a signed/installed build.
#[cfg(feature = "installed")]
pub fn brush_to_paint_trial(
    brush: BrushRef<'_>,
    composite: Composite,
    images: &mut Images,
    _effective_geometry: Affine,
    brush_transform: Affine,
) -> (PaintType, Affine) {
    (brush_to_paint(brush, composite, images), brush_transform)
}

/// Experimental Akula-owned image minification. Never intercepts glifo's
/// glyph atlas or touches non-image brushes. The opt-in flags are sampled
/// once per process, BEFORE the first paint; benchmark processes get separate
/// environments and regular draw calls avoid repeated getenv/allocations.
#[cfg(not(feature = "installed"))]
pub fn brush_to_paint_trial(
    brush: BrushRef<'_>,
    composite: Composite,
    images: &mut Images,
    effective_geometry: Affine,
    brush_transform: Affine,
) -> (PaintType, Affine) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let enabled = *ENABLED.get_or_init(|| {
        std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP").ok().as_deref() == Some("1")
    });
    if !enabled {
        let mut paint = brush_to_paint(brush, composite, images);
        apply_factorial_filter_from_env(&mut paint);
        return (paint, brush_transform);
    }
    static BILINEAR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let use_bilinear = *BILINEAR.get_or_init(|| {
        std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP_FILTER").ok().as_deref() == Some("medium")
    });
    brush_to_paint_candidate_tuned(
        brush, composite, images, effective_geometry, brush_transform, true, use_bilinear,
    )
}

// Four-cell sampler × mipmap experiment: the original image brush may
// already have Medium quality, so a no-op "High→Medium" downgrade is not
// adequate evidence. Force the exact sampler quality in *all four* developer
// processes, then demand a positive native image brush witness.
#[cfg(any(test, not(feature = "installed")))]
fn force_image_sample_quality(paint: &mut PaintType, selection: u8) -> bool {
    let Brush::Image(image) = paint else { return false };
    match selection {
        1 => image.sampler.quality = ImageQuality::High,
        2 => image.sampler.quality = ImageQuality::Medium,
        _ => return false,
    }
    true
}

#[cfg(not(feature = "installed"))]
fn apply_factorial_filter_from_env(paint: &mut PaintType) {
    static FORCE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    let selected = *FORCE.get_or_init(|| {
        match std::env::var("AKULA_EXPERIMENTAL_IMAGE_FILTER_FORCE").ok().as_deref() {
            Some("high") => 1,
            Some("medium") => 2,
            _ => 0,
        }
    });
    if !force_image_sample_quality(paint, selected) {
        return;
    }
    if std::env::var("AKULA_EXPERIMENTAL_IMAGE_FILTER_PROOF").ok().as_deref() == Some("1") {
        static HIGH: std::sync::Once = std::sync::Once::new();
        static MEDIUM: std::sync::Once = std::sync::Once::new();
        match selected {
            1 => HIGH.call_once(|| eprintln!("AKULA_FILTER_FORCED_HIGH_APPLIED")),
            2 => MEDIUM.call_once(|| eprintln!("AKULA_FILTER_FORCED_MEDIUM_APPLIED")),
            _ => {}
        }
    }
}

// Test-only direct activation, independent of environment and OnceLock.
#[cfg(test)]
fn brush_to_paint_candidate(
    brush: BrushRef<'_>,
    composite: Composite,
    images: &mut Images,
    effective_geometry: Affine,
    brush_transform: Affine,
    enabled: bool,
) -> (PaintType, Affine) {
    brush_to_paint_candidate_tuned(
        brush, composite, images, effective_geometry, brush_transform, enabled, false,
    )
}

// Compare the existing bicubic mip candidate to a developer-only bilinear
// candidate without changing users' original ImageBrush quality.
#[cfg(any(test, not(feature = "installed")))]
fn brush_to_paint_candidate_tuned(
    brush: BrushRef<'_>,
    composite: Composite,
    images: &mut Images,
    effective_geometry: Affine,
    brush_transform: Affine,
    enabled: bool,
    use_bilinear: bool,
) -> (PaintType, Affine) {
    let original_id = if let BrushRef::Image(image) = &brush {
        Some(image.image.data.id())
    } else {
        None
    };
    let mut paint = brush_to_paint(brush, composite, images);
    #[cfg(not(feature = "installed"))]
    apply_factorial_filter_from_env(&mut paint);
    if !enabled {
        return (paint, brush_transform);
    }
    let Some(id) = original_id else { return (paint, brush_transform) };
    let Brush::Image(image) = &mut paint else { return (paint, brush_transform) };
    if !matches!(image.sampler.quality, ImageQuality::Medium | ImageQuality::High) {
        #[cfg(not(feature = "installed"))]
        trace_mip_decision(id, "quality", 0, &image.image, effective_geometry, brush_transform);
        return (paint, brush_transform);
    }
    if image.sampler.x_extend != Extend::Pad || image.sampler.y_extend != Extend::Pad {
        #[cfg(not(feature = "installed"))]
        trace_mip_decision(id, "extend", 0, &image.image, effective_geometry, brush_transform);
        return (paint, brush_transform);
    }
    let level = mipmap::level_for_transform(effective_geometry * brush_transform);
    if level == 0 {
        #[cfg(not(feature = "installed"))]
        trace_mip_decision(id, "transform", 0, &image.image, effective_geometry, brush_transform);
        return (paint, brush_transform);
    }
    if let Some(reduced) = images.mip_source(id, &image.image, level) {
        #[cfg(not(feature = "installed"))]
        trace_mip_decision(id, "used", level, &image.image, effective_geometry, brush_transform);
        // A real GUI A/B is invalid without evidence that an image took this path.
        // Emit once only in explicit developer diagnostic mode.
        if !cfg!(feature = "installed")
            && std::env::var("AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF").ok().as_deref() == Some("1")
        {
            static FIRST_APPLIED: std::sync::Once = std::sync::Once::new();
            FIRST_APPLIED.call_once(|| eprintln!("AKULA_MIP_APPLIED level={level}"));
        }
        image.image = reduced;
        if use_bilinear && image.sampler.quality == ImageQuality::High {
            // mipmap::half has already integrated the larger source area.
            // Compare Vello's optimized bilinear path with the current
            // bicubic path; never change Medium/Low or non-mip brushes.
            image.sampler.quality = ImageQuality::Medium;
        }
        // Compensate the reduced image inside the existing brush transform.
        // Geometry, clipping, draw alpha and alignment stay unchanged.
        return (paint, brush_transform * Affine::scale(f64::from(1_u16 << level)));
    }
    #[cfg(not(feature = "installed"))]
    trace_mip_decision(id, "unavailable", level, &image.image, effective_geometry, brush_transform);
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
    // Vello CPU supports exactly one filter primitive per graph. A group
    // with several filters must be encoded as nested layers (filter_chain).
    let [f] = filters else { return None };
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
    Some(VelloFilter::from_primitive(primitive))
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

    #[cfg(feature = "installed")]
    #[test]
    fn installed_build_uses_original_paint_path_without_mipmaps() {
        let brush = test_brush(128, 128);
        let mut images = Images::default();
        let (paint, brush_transform) = brush_to_paint_trial(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            Affine::scale(1.0 / 16.0), Affine::IDENTITY,
        );
        let Brush::Image(image) = paint else { panic!("expected image brush") };
        let ImageSource::Pixmap(original) = image.image else { panic!("expected pixmap") };
        assert_eq!((original.width(), original.height()), (128, 128));
        assert_eq!(brush_transform, Affine::IDENTITY);
        assert!(images.mips.is_empty(), "installed build must not generate mips");
    }

    #[test]
    fn factorial_force_proves_both_samplers_from_a_medium_default() {
        let original = test_brush(128, 129).with_quality(ImageQuality::Medium);
        let mut images = Images::default();
        let mut paint = brush_to_paint(BrushRef::Image(original.as_ref()), Composite::default(), &mut images);
        assert!(force_image_sample_quality(&mut paint, 1));
        let Brush::Image(high) = &paint else { panic!("expected image") };
        assert_eq!(high.sampler.quality, ImageQuality::High);
        let ImageSource::Pixmap(ref high_px) = high.image else { panic!("pixmap expected") };
        assert_eq!((high_px.width(), high_px.height()), (128, 129));
        let high_bytes = high_px.data_as_u8_slice().to_vec();
        assert!(force_image_sample_quality(&mut paint, 2));
        let Brush::Image(medium) = &paint else { panic!("expected image") };
        assert_eq!(medium.sampler.quality, ImageQuality::Medium);
        let ImageSource::Pixmap(ref medium_px) = medium.image else { panic!("pixmap expected") };
        assert_eq!(medium_px.data_as_u8_slice(), high_bytes.as_slice());
        assert!(images.mips.is_empty(), "quality override cannot build mip levels");
        assert!(!force_image_sample_quality(&mut paint, 0));
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
    fn long_lived_mip_reuse_survives_scroll_gap_but_expires_within_cap() {
        let brush = test_brush(128, 128);
        let mut images = Images::default();
        let source = images.source(&brush.image);
        let id = brush.image.data.id();
        let first = images.mip_source_with_policy(
            id, &source, 2, mipmap::MAX_EXTRA_BYTES, true,
        ).expect("initial mipmap");
        let ImageSource::Pixmap(first) = first else { panic!("need pixmap") };
        for _ in 0..384 {
            images.next_frame_with_mip_lifetime(EXPERIMENTAL_MIP_LINGER_FRAMES);
        }
        let second = images.mip_source_with_policy(
            id, &source, 2, mipmap::MAX_EXTRA_BYTES, true,
        ).expect("reuse mipmap after scroll gap");
        let ImageSource::Pixmap(second) = second else { panic!("need pixmap") };
        assert!(Arc::ptr_eq(&first, &second), "must not recompute on image revisit");
        #[cfg(not(feature = "installed"))]
        {
            assert_eq!(images.diag.generated, 2, "no build after 384-frame absence");
        }
        for _ in 0..EXPERIMENTAL_MIP_LINGER_FRAMES {
            images.next_frame_with_mip_lifetime(EXPERIMENTAL_MIP_LINGER_FRAMES);
        }
        assert!(images.mips.is_empty(), "unused mips age out after 512 frames");
    }

    #[test]
    fn long_lived_cache_eviction_frees_lru_atomically_under_fixed_budget() {
        let first = test_brush(128, 128);
        let second = test_brush(128, 128);
        let mut images = Images::default();
        let a = images.source(&first.image);
        let b = images.source(&second.image);
        let aid = first.image.data.id();
        let bid = second.image.data.id();
        assert_ne!(aid, bid);
        let budget = 64 * 64 * 4 + 32 * 32 * 4;
        assert!(images.mip_source_with_policy(aid, &a, 2, budget, true).is_some());
        assert_eq!(images.mips.len(), 2);

        // A request that can NEVER fit must leave the previous cache intact.
        assert!(images.mip_source_with_policy(bid, &b, 2, budget - 1, true).is_none());
        assert!(images.mips.contains_key(&(aid, 2)));
        assert!(!images.mips.contains_key(&(bid, 1)));

        // At full size, evict A to give B its fair bounded share.
        assert!(images.mip_source_with_policy(bid, &b, 2, budget, true).is_some());
        assert_eq!(images.mips.len(), 2);
        assert!(!images.mips.contains_key(&(aid, 1)));
        assert!(!images.mips.contains_key(&(aid, 2)));
        assert!(images.mips.contains_key(&(bid, 2)));
        let bytes: usize = images.mips.values()
            .map(|(p, _)| usize::from(p.width()) * usize::from(p.height()) * 4)
            .sum();
        assert!(bytes <= budget);
    }

    #[test]
    fn long_lived_multi_image_churn_obeys_budget_and_reuses_recent_entries() {
        // Exercise the exact 32-MiB policy under a tiny budget, avoiding
        // enormous synthetic images on a shared CI runner.
        const MIP_BYTES: usize = (64 * 64 + 32 * 32) * 4;
        const BUDGET: usize = MIP_BYTES * 5;
        let brushes: Vec<_> = (0..16).map(|_| test_brush(128, 128)).collect();
        let mut images = Images::default();
        for turn in 0..3 {
            for brush in &brushes {
                images.next_frame_with_mip_lifetime(EXPERIMENTAL_MIP_LINGER_FRAMES);
                let source = images.source(&brush.image);
                let id = brush.image.data.id();
                let mip = images.mip_source_with_policy(id, &source, 2, BUDGET, true);
                assert!(mip.is_some(), "bounded LRU failed at turn={turn}");
                let used: usize = images.mips.values()
                    .map(|(p, _)| usize::from(p.width()) * usize::from(p.height()) * 4)
                    .sum();
                assert!(used <= BUDGET, "cache exceeded hard budget: {used}");
                assert!(images.mips.contains_key(&(id, 2)));
            }
        }
        assert!(!images.mips.contains_key(&(brushes[0].image.data.id(), 2)));
        assert!(images.mips.contains_key(&(brushes[15].image.data.id(), 2)));
    }

    #[test]
    fn optional_bilinear_is_applied_only_after_successful_prefilter() {
        let brush = test_brush(128, 128);
        let mut images = Images::default();
        let (base, _) = brush_to_paint_candidate_tuned(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            Affine::IDENTITY, Affine::IDENTITY, true, true,
        );
        let Brush::Image(base) = base else { panic!("image expected") };
        assert_eq!(base.sampler.quality, ImageQuality::High, "no mip, no downgrade");

        let (mipped, transform) = brush_to_paint_candidate_tuned(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            Affine::scale(1.0 / 8.0), Affine::IDENTITY, true, true,
        );
        let Brush::Image(mipped) = mipped else { panic!("image expected") };
        assert_eq!(mipped.sampler.quality, ImageQuality::Medium);
        assert_eq!(transform, Affine::scale(4.0));
        assert_eq!(brush.sampler.quality, ImageQuality::High, "original brush unchanged");

        let (disabled, transform) = brush_to_paint_candidate_tuned(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            Affine::scale(1.0 / 8.0), Affine::IDENTITY, false, true,
        );
        let Brush::Image(disabled) = disabled else { panic!("image expected") };
        assert_eq!(disabled.sampler.quality, ImageQuality::High);
        assert_eq!(transform, Affine::IDENTITY);
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

    fn draw(data: &[u8], ratio: u16, scene: Scene, enabled: bool, bilinear: bool, workers: u16) -> (Pixmap, usize) {
        let brush = crate::render::pixels::from_rgba(
            data.to_vec(), SIDE as u32, SIDE as u32, ImageAlphaType::Alpha,
        )
        .expect("valid source fixture")
        .with_quality(ImageQuality::High);

        let transform = Affine::scale(1.0 / f64::from(ratio));
        let mut images = Images::default();
        let (paint, brush_transform) = brush_to_paint_candidate_tuned(
            BrushRef::Image(brush.as_ref()), Composite::default(),
            &mut images, transform, Affine::IDENTITY, enabled, bilinear,
        );
        if let Brush::Image(image) = &paint {
            let expected = if enabled && bilinear { ImageQuality::Medium } else { ImageQuality::High };
            assert_eq!(image.sampler.quality, expected);
        } else {
            panic!("expected image brush");
        }
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
                    let (before, _) = draw(&data, ratio, scene, false, false, workers);
                    let (base_mae, base_max, pixels) =
                        error_against_area(&data, &before, ratio, scene);
                    for bilinear in [false, true] {
                        let (after, mips) = draw(&data, ratio, scene, true, bilinear, workers);
                        let (trial_mae, trial_max, _) =
                            error_against_area(&data, &after, ratio, scene);

                        if matches!(scene, Scene::AlphaCrop) {
                            let corner = after.data()[0];
                            assert_eq!(
                                [corner.r, corner.g, corner.b, corner.a],
                                [255, 255, 255, 255], "crop moved under sampler switch"
                            );
                        }
                        println!(
                            "AKULA_MIP_REALPNG scene={scene:?} ratio={ratio} workers={workers} \
                             filter={} baseline_mae={base_mae:.3} candidate_mae={trial_mae:.3} \
                             baseline_max={base_max} candidate_max={trial_max} \
                             compared_channels={pixels} allocated_mips={mips}",
                            if bilinear { "bilinear" } else { "bicubic" },
                        );
                        assert!(
                            trial_mae <= base_mae + 4.0 && trial_mae <= 25.0,
                            "candidate quality regression scene={scene:?} ratio={ratio} \
                             workers={workers} bilinear={bilinear} \
                             baseline_mae={base_mae:.3} candidate_mae={trial_mae:.3}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 24);
    }
}


#[cfg(test)]
mod mipmap_stage_benchmarks {
    use super::*;
    use std::time::Instant;
    use masonry::peniko::{ImageAlphaType, ImageQuality};
    use vello_cpu::color::palette::css::WHITE;
    use vello_cpu::{RasterizerSettings, RenderContext, RenderSettings, Resources, TargetInit};

    fn stats_us(samples: &[f64]) -> (f64, f64) {
        assert!(!samples.is_empty());
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
        (median, p95)
    }

    /// Diagnostic: separate Paint/Source/Mip conversion from actual CPU
    /// rasterization, with the same PNG and 200px canvas in every mode.
    /// Kept ignored to avoid noisy performance thresholds in normal CI.
    #[test]
    #[ignore]
    fn real_photo_mip_paint_vs_raster_stages() {
        const WARMUP: usize = 20;
        const SAMPLES: usize = 180;
        let rgba = image::load_from_memory(
            include_bytes!("../../../assets/city-cover.png"),
        )
        .expect("pinned photo PNG")
        .to_rgba8();
        let (width, height) = rgba.dimensions();
        assert_eq!((width, height), (1280, 884));
        let pixels = rgba.into_raw();
        let scale = Affine::scale(200.0 / f64::from(width));
        let shape = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));

        for workers in [0u16, 2u16] {
            for (mode, enable_mip, bilinear) in [
                ("baseline", false, false),
                ("mip_high", true, false),
                ("mip_medium", true, true),
            ] {
                let brush = crate::render::pixels::from_rgba(
                    pixels.clone(), width, height, ImageAlphaType::Alpha,
                )
                .expect("valid real PNG")
                .with_quality(ImageQuality::High);

                let mut images = Images::default();
                let mut render = RenderContext::new_with(
                    200, 139,
                    RenderSettings { num_threads: workers, ..RenderSettings::default() },
                );
                let mut resources = Resources::default();
                let mut canvas = Pixmap::new(200, 139);
                let mut convert_samples = Vec::with_capacity(SAMPLES);
                let mut encode_samples = Vec::with_capacity(SAMPLES);
                let mut raster_samples = Vec::with_capacity(SAMPLES);
                let mut first_frame_ms = None;

                for iteration in 0..(WARMUP + SAMPLES) {
                    images.next_frame();
                    let started = Instant::now();
                    let (paint, brush_transform) = brush_to_paint_candidate_tuned(
                        BrushRef::Image(brush.as_ref()), Composite::default(),
                        &mut images, scale, Affine::IDENTITY, enable_mip, bilinear,
                    );
                    let convert_us = started.elapsed().as_secs_f64() * 1e6;

                    let encode_start = Instant::now();
                    render.reset();
                    render.set_transform(scale);
                    render.set_paint_transform(brush_transform);
                    render.set_paint(paint);
                    render.fill_rect(&shape);
                    render.flush();
                    let encode_us = encode_start.elapsed().as_secs_f64() * 1e6;

                    let raster_start = Instant::now();
                    render.render_with(
                        &mut canvas, &mut resources,
                        RasterizerSettings {
                            target_init: TargetInit::Clear(WHITE),
                            ..RasterizerSettings::default()
                        },
                    );
                    let raster_us = raster_start.elapsed().as_secs_f64() * 1e6;
                    if iteration == 0 {
                        first_frame_ms = Some((convert_us + encode_us + raster_us) / 1000.0);
                    }
                    if iteration >= WARMUP {
                        convert_samples.push(convert_us);
                        encode_samples.push(encode_us);
                        raster_samples.push(raster_us);
                    }
                }

                let (convert_p50, convert_p95) = stats_us(&convert_samples);
                let (encode_p50, encode_p95) = stats_us(&encode_samples);
                let (raster_p50, raster_p95) = stats_us(&raster_samples);
                // The direct-hit fast path intentionally keeps the requested
                // final level warm while intermediate levels expire after two
                // frames; a healthy warm cache therefore holds ONE level.
                let expected_levels = if enable_mip { 1 } else { 0 };
                assert_eq!(
                    images.mips.len(), expected_levels,
                    "final mip must remain warm without retaining intermediates",
                );
                println!(
                    "AKULA_MIP_STAGES workers={workers} mode={mode} \
                     first_frame_ms={:.3} convert_p50_us={convert_p50:.2} \
                     convert_p95_us={convert_p95:.2} encode_p50_us={encode_p50:.2} \
                     encode_p95_us={encode_p95:.2} raster_p50_us={raster_p50:.2} \
                     raster_p95_us={raster_p95:.2} cached_levels={}",
                    first_frame_ms.unwrap_or_default(),
                    images.mips.len(),
                );
            }
        }
    }
}


// Independent fractional-footprint reference for odd real-photo dimensions.
// Unlike mipmap::half, expected output integrates the actual ORIGINAL RGBA.
#[cfg(test)]
mod fractional_odd_quality_tests {
    use super::*;
    use masonry::peniko::{ImageAlphaType, ImageQuality};
    use vello_cpu::color::palette::css::WHITE;
    use vello_cpu::{RasterizerSettings, RenderContext, RenderSettings, Resources, TargetInit};

    fn input(w: u16, h: u16, alpha: bool) -> Vec<u8> {
        let png = image::load_from_memory(include_bytes!("../../../assets/city-cover.png"))
            .expect("pinned cover PNG").to_rgba8();
        let mut rgba = image::imageops::resize(
            &png, u32::from(w), u32::from(h), image::imageops::FilterType::Lanczos3,
        ).into_raw();
        if alpha {
            for y in 0..usize::from(h) {
                for x in 0..usize::from(w) {
                    let i = (y * usize::from(w) + x) * 4;
                    // Nonzero RGB under zero alpha catches straight-alpha halos.
                    rgba[i + 3] = [0, 64, 128, 192, 255][(x / 7 + y / 9) % 5];
                }
            }
        }
        rgba
    }

    fn render(src: &[u8], w: u16, h: u16, scale: f64,
              enabled: bool, workers: u16) -> Pixmap {
        let brush = crate::render::pixels::from_rgba(
            src.to_vec(), u32::from(w), u32::from(h), ImageAlphaType::Alpha,
        ).expect("RGBA image").with_quality(ImageQuality::High);
        let geometry = Affine::scale(scale);
        let mut images = Images::default();
        let (paint, brush_transform) = brush_to_paint_candidate_tuned(
            BrushRef::Image(brush.as_ref()), Composite::default(),
            &mut images, geometry, Affine::IDENTITY, enabled, true,
        );
        assert_eq!(images.mips.is_empty(), !enabled, "mip path was not exercised");
        let Brush::Image(painted) = &paint else { panic!("expected image") };
        assert_eq!(
            painted.sampler.quality,
            if enabled { ImageQuality::Medium } else { ImageQuality::High },
        );
        let cw = (f64::from(w) * scale).ceil() as u16 + 2;
        let ch = (f64::from(h) * scale).ceil() as u16 + 2;
        let mut ctx = RenderContext::new_with(
            cw, ch, RenderSettings { num_threads: workers, ..RenderSettings::default() },
        );
        ctx.set_transform(geometry);
        ctx.set_paint_transform(brush_transform);
        ctx.set_paint(paint);
        ctx.fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        ctx.flush();
        let mut out = Pixmap::new(cw, ch);
        ctx.render_with(
            &mut out, &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..RasterizerSettings::default()
            },
        );
        // No transformed geometry may contaminate the extra white canvas.
        let border = out.data()[usize::from(ch - 1) * usize::from(cw) + usize::from(cw - 1)];
        assert_eq!([border.r, border.g, border.b, border.a], [255, 255, 255, 255]);
        out
    }

    fn exact_pixel(src: &[u8], w: usize, h: usize, x: usize, y: usize,
                   scale: f64) -> [f64; 3] {
        let u0 = (x as f64 / scale).clamp(0.0, w as f64);
        let u1 = ((x + 1) as f64 / scale).clamp(0.0, w as f64);
        let v0 = (y as f64 / scale).clamp(0.0, h as f64);
        let v1 = ((y + 1) as f64 / scale).clamp(0.0, h as f64);
        let area = (u1 - u0) * (v1 - v0);
        assert!(area > 0.0);
        let mut premul = [0.0f64; 3];
        let mut alpha_sum = 0.0;
        for iy in (v0.floor() as usize)..(v1.ceil() as usize).min(h) {
            let wy = (v1.min((iy + 1) as f64) - v0.max(iy as f64)).max(0.0);
            for ix in (u0.floor() as usize)..(u1.ceil() as usize).min(w) {
                let wx = (u1.min((ix + 1) as f64) - u0.max(ix as f64)).max(0.0);
                let weight = wx * wy;
                let i = (iy * w + ix) * 4;
                let alpha = f64::from(src[i + 3]) / 255.0;
                for channel in 0..3 {
                    premul[channel] += weight * alpha * f64::from(src[i + channel]);
                }
                alpha_sum += weight * alpha;
            }
        }
        std::array::from_fn(|channel| {
            premul[channel] / area + 255.0 * (1.0 - alpha_sum / area)
        })
    }

    fn quality(src: &[u8], w: u16, h: u16, scale: f64, frame: &Pixmap) -> (f64, f64) {
        let stride = usize::from(frame.width());
        let max_x = (f64::from(w) * scale).floor() as usize;
        let max_y = (f64::from(h) * scale).floor() as usize;
        let (mut total, mut worst, mut count) = (0.0f64, 0.0f64, 0usize);
        // Compare the image interior; destination clipping is checked above.
        for y in 2..max_y.saturating_sub(2) {
            for x in 2..max_x.saturating_sub(2) {
                let expected = exact_pixel(
                    src, usize::from(w), usize::from(h), x, y, scale,
                );
                let pixel = frame.data()[y * stride + x];
                for (actual, wanted) in [pixel.r, pixel.g, pixel.b].into_iter().zip(expected) {
                    let error = (f64::from(actual) - wanted).abs();
                    total += error;
                    worst = worst.max(error);
                    count += 1;
                }
            }
        }
        assert!(count > 0);
        (total / count as f64, worst)
    }

    #[test]
    fn fractional_photo_alpha_and_odd_dimensions_have_bounded_area_error() {
        let mut cases = 0;
        for (w, h) in [(127u16, 129u16), (131, 89), (128, 127)] {
            for (scale, alpha) in [(0.241, false), (0.315, true), (0.413, true)] {
                let rgba = input(w, h, alpha);
                for workers in [0u16, 2u16] {
                    let baseline = render(&rgba, w, h, scale, false, workers);
                    let candidate = render(&rgba, w, h, scale, true, workers);
                    let (base_mae, _) = quality(&rgba, w, h, scale, &baseline);
                    let (mip_mae, mip_worst) = quality(&rgba, w, h, scale, &candidate);
                    println!(
                        "AKULA_MIP_FRACTIONAL size={w}x{h} scale={scale:.3} alpha={alpha} \
                         workers={workers} baseline_mae={base_mae:.3} \
                         mip_mae={mip_mae:.3} mip_worst={mip_worst:.3}"
                    );
                    assert!(
                        mip_mae <= base_mae + 4.0 && mip_mae <= 12.0,
                        "fractional/odd mip area regression: {w}x{h} \
                         scale={scale} alpha={alpha} workers={workers}: \
                         original={base_mae:.3} candidate={mip_mae:.3}"
                    );
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 18);
    }
}


// Adversarial alpha/clip scene following the same brush transform and rounded
// crop semantics as ui::editor::embed::media::paint::draw_image.
// Transparent saturated RGB must never bleed onto a white background.
#[cfg(test)]
mod mipmap_crop_alpha_edge_tests {
    use super::*;
    use masonry::peniko::{ImageAlphaType, ImageQuality};
    use vello_cpu::color::palette::css::WHITE;
    use vello_cpu::kurbo::{RoundedRect, Shape as _};
    use vello_cpu::{RasterizerSettings, RenderContext, RenderSettings, Resources, TargetInit};

    const W: u16 = 127;
    const H: u16 = 129;

    fn alpha_edge_fixture() -> Vec<u8> {
        let mut rgba = Vec::with_capacity(usize::from(W) * usize::from(H) * 4);
        for y in 0..usize::from(H) {
            for x in 0..usize::from(W) {
                let distance = (x.min(126 - x)).min(y.min(128 - y));
                let alpha = if (27..100).contains(&x) && (23..108).contains(&y) {
                    255
                } else if distance < 9 {
                    0
                } else {
                    [0, 64, 128, 192][(x + y) % 4]
                };
                // Fully transparent magenta is deliberately adversarial;
                // every VISIBLE texel is achromatic black.
                if alpha == 0 {
                    rgba.extend_from_slice(&[255, 0, 255, 0]);
                } else {
                    rgba.extend_from_slice(&[0, 0, 0, alpha]);
                }
            }
        }
        rgba
    }

    fn render(scale: f64, rounded: bool, enabled: bool, workers: u16) -> Pixmap {
        let brush = crate::render::pixels::from_rgba(
            alpha_edge_fixture(), u32::from(W), u32::from(H), ImageAlphaType::Alpha,
        )
        .expect("valid RGBA")
        .with_quality(ImageQuality::High);
        let original_id = brush.image.data.id();
        let image_transform = Affine::translate((14.75, 8.35)) * Affine::scale(scale);
        let mut images = Images::default();
        let (paint, brush_transform) = brush_to_paint_candidate_tuned(
            BrushRef::Image(brush.as_ref()), Composite::default(), &mut images,
            image_transform, Affine::IDENTITY, enabled, true,
        );
        assert_eq!(images.mips.is_empty(), !enabled);
        assert_eq!(brush.image.data.id(), original_id, "converter mutated source");
        let Brush::Image(image) = &paint else { panic!("image required") };
        assert_eq!(
            image.sampler.quality,
            if enabled { ImageQuality::Medium } else { ImageQuality::High },
        );

        let clip = Rect::new(24.0, 18.0, 44.0, 36.0);
        let mut ctx = RenderContext::new_with(
            64, 56, RenderSettings { num_threads: workers, ..RenderSettings::default() },
        );
        ctx.set_transform(Affine::IDENTITY);
        if rounded {
            let clip_path = RoundedRect::from_rect(clip, 5.0).to_path(0.1);
            ctx.push_clip_path(&clip_path);
        } else {
            ctx.push_clip_rect(&clip);
        }
        ctx.set_transform(image_transform);
        ctx.set_paint_transform(brush_transform);
        ctx.set_paint(paint);
        ctx.fill_rect(&Rect::new(0.0, 0.0, f64::from(W), f64::from(H)));
        ctx.pop_clip();
        ctx.flush();
        let mut out = Pixmap::new(64, 56);
        ctx.render_with(
            &mut out, &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..RasterizerSettings::default()
            },
        );
        out
    }

    #[test]
    fn transparent_chroma_does_not_bleed_through_real_crop_or_rounded_clip() {
        let mut checked = 0;
        for scale in [0.241, 0.313] {
            for rounded in [false, true] {
                for workers in [0, 2] {
                    for enabled in [false, true] {
                        let frame = render(scale, rounded, enabled, workers);
                        let mut darkest = 255u8;
                        let mut worst_chroma_spread = 0u8;
                        for y in 0..usize::from(frame.height()) {
                            for x in 0..usize::from(frame.width()) {
                                let p = frame.data()[y * usize::from(frame.width()) + x];
                                let min_rgb = p.r.min(p.g).min(p.b);
                                let max_rgb = p.r.max(p.g).max(p.b);
                                worst_chroma_spread = worst_chroma_spread.max(max_rgb - min_rgb);
                                if (x <= 22 || x >= 46) || (y <= 16 || y >= 38) {
                                    assert_eq!(
                                        [p.r, p.g, p.b, p.a], [255, 255, 255, 255],
                                        "cropped image leaked outside frame at ({x},{y})"
                                    );
                                } else {
                                    darkest = darkest.min(min_rgb);
                                }
                            }
                        }
                        assert!(darkest <= 64, "test rendered no opaque black content");
                        assert!(
                            worst_chroma_spread <= 2,
                            "transparent RGB created coloured halo: scale={scale} \
                             rounded={rounded} enabled={enabled} workers={workers} \
                             max_channel_spread={worst_chroma_spread}"
                        );
                        println!(
                            "AKULA_MIP_CROP_CHROMA scale={scale} rounded={rounded} \
                             enabled={enabled} workers={workers} \
                             darkest={darkest} max_chroma_spread={worst_chroma_spread}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 16);
    }
}


// Independent coverage check for the exact 8px rounded image-frame radius used
// by the editor. The expected mask is a supersampled *analytic shape*, not a
// Vello output or a second invocation of the same clip renderer.
#[cfg(test)]
mod mipmap_rounded_coverage_tests {
    use super::*;
    use masonry::peniko::{ImageAlphaType, ImageQuality};
    use vello_cpu::color::palette::css::WHITE;
    use vello_cpu::kurbo::{RoundedRect, Shape as _};
    use vello_cpu::{RasterizerSettings, RenderContext, RenderSettings, Resources, TargetInit};

    const WIDTH: u16 = 127;
    const HEIGHT: u16 = 129;
    const FRAME_W: u16 = 64;
    const FRAME_H: u16 = 56;

    fn draw(scale: f64, workers: u16, enabled: bool) -> Pixmap {
        // Opaque black on all source pixels isolates *geometric coverage*
        // from image colour/alpha interpolation. Includes fractional source
        // translation and odd source dimensions.
        let raw = vec![0u8, 0, 0, 255].repeat(usize::from(WIDTH) * usize::from(HEIGHT));
        let brush = crate::render::pixels::from_rgba(
            raw, u32::from(WIDTH), u32::from(HEIGHT), ImageAlphaType::Alpha,
        )
        .expect("valid uniform RGBA image")
        .with_quality(ImageQuality::High);
        let geometry = Affine::translate((14.75, 8.35)) * Affine::scale(scale);
        let mut images = Images::default();
        let (paint, paint_transform) = brush_to_paint_candidate_tuned(
            BrushRef::Image(brush.as_ref()), Composite::default(),
            &mut images, geometry, Affine::IDENTITY, enabled, true,
        );
        assert_eq!(images.mips.is_empty(), !enabled);
        let clip = RoundedRect::from_rect(
            Rect::new(24.0, 18.0, 44.0, 36.0),
            8.0, // ui/editor/embed/media/paint.rs: RADIUS
        );
        let mut ctx = RenderContext::new_with(
            FRAME_W, FRAME_H,
            RenderSettings { num_threads: workers, ..RenderSettings::default() },
        );
        ctx.set_transform(Affine::IDENTITY);
        ctx.push_clip_path(&clip.to_path(0.1));
        ctx.set_transform(geometry);
        ctx.set_paint_transform(paint_transform);
        ctx.set_paint(paint);
        ctx.fill_rect(&Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)));
        ctx.pop_clip();
        ctx.flush();

        let mut output = Pixmap::new(FRAME_W, FRAME_H);
        ctx.render_with(
            &mut output, &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..RasterizerSettings::default()
            },
        );
        output
    }

    fn inside_rounded(x: f64, y: f64) -> bool {
        // Signed-distance geometry for rounded rectangle [24,44] x [18,36]
        // with radius 8: clamp to its [32,36] x [26,28] core, then compare
        // distance to 8. This is independent of kurbo's path tessellation.
        let dx = x - x.clamp(32.0, 36.0);
        let dy = y - y.clamp(26.0, 28.0);
        dx * dx + dy * dy <= 64.0
    }

    fn coverage(x: usize, y: usize) -> f64 {
        const SAMPLES: usize = 16;
        let mut hit = 0usize;
        for sy in 0..SAMPLES {
            for sx in 0..SAMPLES {
                let px = x as f64 + (sx as f64 + 0.5) / SAMPLES as f64;
                let py = y as f64 + (sy as f64 + 0.5) / SAMPLES as f64;
                if inside_rounded(px, py) {
                    hit += 1;
                }
            }
        }
        hit as f64 / (SAMPLES * SAMPLES) as f64
    }

    fn encode_srgb(linear: f64) -> f64 {
        if linear <= 0.003_130_8 {
            12.92 * linear
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        }
    }

    #[test]
    fn rounded_image_clip_matches_independent_coverage_on_both_sampler_paths() {
        let mut checked = 0;
        for scale in [0.241, 0.313] {
            for workers in [0, 2] {
                let before = draw(scale, workers, false);
                let after = draw(scale, workers, true);
                let mut transition_pixels = 0;
                let mut largest_deviation = 0.0f64;
                let mut largest_mip_delta = 0u8;

                for y in 0..usize::from(FRAME_H) {
                    for x in 0..usize::from(FRAME_W) {
                        let index = y * usize::from(FRAME_W) + x;
                        let cov = coverage(x, y);
                        let baseline = before.data()[index];
                        let candidate = after.data()[index];
                        for p in [baseline, candidate] {
                            assert_eq!(p.r, p.g, "colored channel from black/white clip");
                            assert_eq!(p.g, p.b, "colored channel from black/white clip");
                            assert_eq!(p.a, 255, "transparent output frame");
                            let intensity = f64::from(p.r);
                            if cov == 0.0 {
                                assert!(intensity >= 252.0, "leak at ({x},{y})");
                            } else if cov == 1.0 {
                                assert!(intensity <= 3.0, "missing fill at ({x},{y})");
                            } else {
                                // Vello may composite in linear or encoded sRGB;
                                // both accepted colour-space paths must respect
                                // the analytic coverage interval with AA slack.
                                let linear = 255.0 * (1.0 - cov);
                                let gamma = 255.0 * encode_srgb(1.0 - cov);
                                let deviation = if intensity < linear.min(gamma) {
                                    linear.min(gamma) - intensity
                                } else if intensity > linear.max(gamma) {
                                    intensity - linear.max(gamma)
                                } else {
                                    0.0
                                };
                                largest_deviation = largest_deviation.max(deviation);
                                assert!(
                                    deviation <= 24.0,
                                    "incorrect rounded AA at ({x},{y}): coverage={cov:.3}, \
                                     intensity={intensity:.1}, deviation={deviation:.1}, \
                                     scale={scale}, workers={workers}"
                                );
                                if cov > 0.1 && cov < 0.9 {
                                    transition_pixels += 1;
                                }
                            }
                        }
                        let mip_delta = baseline.r.abs_diff(candidate.r);
                        largest_mip_delta = largest_mip_delta.max(mip_delta);
                        assert!(
                            mip_delta <= 5,
                            "mipmap changed opaque rounded-clip geometry at ({x},{y}): \
                             delta={mip_delta}, scale={scale}, workers={workers}"
                        );
                    }
                }
                assert!(
                    transition_pixels >= 12,
                    "test failed to exercise enough antialiased rounded-corner pixels"
                );
                println!(
                    "AKULA_MIP_ROUND_ANALYTIC scale={scale} workers={workers} \
                     transition_pixels={transition_pixels} \
                     worst_reference_deviation={largest_deviation:.2} \
                     worst_mip_delta={largest_mip_delta}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 4);
    }
}

#[cfg(test)]
#[path = "convert_adaptive_lod_tests.rs"]
mod adaptive_lod_tests;
