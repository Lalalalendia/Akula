use std::io::Cursor;

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub natural: (u32, u32),
}

pub fn fit(natural: (u32, u32), limit: u32, max_pixels: f64) -> (u32, u32) {
    let (w, h) = (f64::from(natural.0), f64::from(natural.1));
    let k = (f64::from(limit) / w).min((max_pixels / (w * h)).sqrt());
    if k >= 1.0 {
        return natural;
    }
    ((w * k).round().clamp(1.0, w) as u32, (h * k).round().clamp(1.0, h) as u32)
}

pub fn decode(bytes: &[u8], limit: u32, max_pixels: f64) -> Option<Decoded> {
    let fitted = |natural| fit(natural, limit, max_pixels);
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => png(bytes, fitted),
        image::ImageFormat::Jpeg => jpeg(bytes, fitted),
        _ => None,
    }
    .or_else(|| any(bytes, fitted))
}

fn png(bytes: &[u8], fitted: impl Fn((u32, u32)) -> (u32, u32)) -> Option<Decoded> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if info.interlaced {
        return None;
    }
    let natural = (info.width, info.height);
    let channels = reader.output_color_type().0.samples();
    let mut shrink = Shrink::new(natural, fitted(natural));
    while let Some(row) = reader.next_row().ok()? {
        shrink.push(row.data(), channels);
    }
    shrink.finish(natural)
}

fn jpeg(bytes: &[u8], fitted: impl Fn((u32, u32)) -> (u32, u32)) -> Option<Decoded> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    let natural = (u32::from(info.width), u32::from(info.height));
    let target = fitted(natural);
    let (w, h) = if target != natural {
        decoder.scale(target.0 as u16, target.1 as u16).ok()?
    } else {
        (info.width, info.height)
    };
    let pixels = decoder.decode().ok()?;
    let channels = match decoder.info()?.pixel_format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::RGB24 => 3,
        _ => return None,
    };
    let from = (u32::from(w), u32::from(h));
    let mut shrink = Shrink::new(from, (target.0.min(from.0), target.1.min(from.1)));
    for row in pixels.chunks_exact(from.0 as usize * channels) {
        shrink.push(row, channels);
    }
    shrink.finish(natural)
}

fn any(bytes: &[u8], fitted: impl Fn((u32, u32)) -> (u32, u32)) -> Option<Decoded> {
    let mut image = image::load_from_memory(bytes).ok()?;
    let natural = (image.width(), image.height());
    let (w, h) = fitted(natural);
    if (w, h) != natural {
        image = image.thumbnail_exact(w, h);
    }
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(Decoded { width, height, pixels: rgba.into_raw(), natural })
}

fn rgba(px: &[u8]) -> [u8; 4] {
    match *px {
        [l] => [l, l, l, 255],
        [l, a] => [l, l, l, a],
        [r, g, b] => [r, g, b, 255],
        [r, g, b, a, ..] => [r, g, b, a],
        [] => [0; 4],
    }
}

struct Shrink {
    to: (u32, u32),
    from_height: u32,
    columns: Vec<u32>,
    sums: Vec<u64>,
    row: u32,
    out: Vec<u8>,
}

impl Shrink {
    fn new(from: (u32, u32), to: (u32, u32)) -> Shrink {
        let columns = (0..from.0).map(|x| (u64::from(x) * u64::from(to.0) / u64::from(from.0)) as u32).collect();
        Shrink {
            to,
            from_height: from.1,
            columns,
            sums: vec![0; to.0 as usize * 5],
            row: 0,
            out: Vec::with_capacity(to.0 as usize * to.1 as usize * 4),
        }
    }

    fn push(&mut self, row: &[u8], channels: usize) {
        if (self.columns.len() as u32, self.from_height) == self.to {
            if channels == 4 {
                self.out.extend_from_slice(row);
            } else {
                self.out.extend(row.chunks_exact(channels).flat_map(rgba));
            }
            return;
        }
        for (px, &column) in row.chunks_exact(channels).zip(&self.columns) {
            let [r, g, b, a] = rgba(px);
            let a = u64::from(a);
            let at = column as usize * 5;
            let sum = &mut self.sums[at..at + 5];
            sum[0] += u64::from(r) * a;
            sum[1] += u64::from(g) * a;
            sum[2] += u64::from(b) * a;
            sum[3] += a;
            sum[4] += 1;
        }
        let line = |y: u32| u64::from(y) * u64::from(self.to.1) / u64::from(self.from_height);
        self.row += 1;
        if line(self.row) != line(self.row - 1) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        for sum in self.sums.chunks_exact_mut(5) {
            let (alpha, count) = (sum[3], sum[4].max(1));
            let pixel = if alpha == 0 {
                [0; 4]
            } else {
                let channel = |s: u64| ((s + alpha / 2) / alpha) as u8;
                [channel(sum[0]), channel(sum[1]), channel(sum[2]), ((alpha + count / 2) / count) as u8]
            };
            self.out.extend_from_slice(&pixel);
            sum.fill(0);
        }
    }

    fn finish(self, natural: (u32, u32)) -> Option<Decoded> {
        let (width, height) = self.to;
        (self.out.len() == width as usize * height as usize * 4).then_some(Decoded { width, height, pixels: self.out, natural })
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, fit};

    fn encode(image: &image::RgbaImage, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        match format {
            image::ImageFormat::Jpeg => image::DynamicImage::ImageRgba8(image.clone()).into_rgb8().write_to(&mut std::io::Cursor::new(&mut bytes), format).unwrap(),
            _ => image.write_to(&mut std::io::Cursor::new(&mut bytes), format).unwrap(),
        }
        bytes
    }

    #[test]
    fn large_images_are_decoded_straight_to_the_shown_size() {
        let image = image::RgbaImage::from_fn(1000, 600, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, if x < 500 { 255 } else { 0 }]));
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg, image::ImageFormat::WebP] {
            let decoded = decode(&encode(&image, format), 300, f64::INFINITY).unwrap();
            assert_eq!((decoded.width, decoded.height, decoded.natural), (300, 180, (1000, 600)), "{format:?}");
            assert_eq!(decoded.pixels.len(), 300 * 180 * 4);
        }
        let png = decode(&encode(&image, image::ImageFormat::Png), 300, f64::INFINITY).unwrap();
        let pixel = |x: usize, y: usize| &png.pixels[(y * 300 + x) * 4..][..4];
        assert_eq!(pixel(10, 10)[3], 255);
        assert_eq!(pixel(290, 10), [0, 0, 0, 0]);
        let small = decode(&encode(&image, image::ImageFormat::Png), 2000, f64::INFINITY).unwrap();
        assert_eq!((small.width, small.height), (1000, 600));
        assert_eq!(&small.pixels[..8], &image.as_raw()[..8]);
    }

    #[test]
    fn transparent_edges_keep_their_color() {
        let image = image::RgbaImage::from_fn(4, 2, |x, _| if x % 2 == 0 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 0, 0]) });
        let decoded = decode(&encode(&image, image::ImageFormat::Png), 2, f64::INFINITY).unwrap();
        assert_eq!(&decoded.pixels[..4], [255, 0, 0, 128]);
    }

    #[test]
    fn full_resolution_jpeg_uses_native_decoder_without_losing_dimensions() {
        let sample = image::RgbaImage::from_fn(64, 48, |x, y| {
            image::Rgba([(x * 3) as u8, (y * 4) as u8, 80, 255])
        });
        let data = encode(&sample, image::ImageFormat::Jpeg);
        let native = super::jpeg(&data, |natural| natural)
            .expect("JPEG decoder should support unscaled images");
        let reference = super::any(&data, |natural| natural)
            .expect("fallback decoder should support the same image");
        assert_eq!((native.width, native.height, native.natural), (64, 48, (64, 48)));
        assert_eq!((reference.width, reference.height, reference.natural), (64, 48, (64, 48)));
        assert_eq!(native.pixels.len(), reference.pixels.len());
        assert!(native.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn native_rgba_scanlines_are_preserved_without_resizing() {
        let original = [100, 40, 10, 255, 20, 70, 200, 37];
        let mut shrink = super::Shrink::new((2, 1), (2, 1));
        shrink.push(&original, 4);
        let decoded = shrink.finish((2, 1)).unwrap();
        assert_eq!(decoded.pixels, original);
    }

    #[test]
    #[ignore = "run in image performance workflow"]
    fn benchmark_unscaled_jpeg_direct_vs_fallback() {
        use std::hint::black_box;
        use std::time::Instant;

        let image = image::RgbaImage::from_fn(1024, 768, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
        });
        let bytes = encode(&image, image::ImageFormat::Jpeg);
        let native = super::jpeg(&bytes, |size| size).unwrap();
        let reference = super::any(&bytes, |size| size).unwrap();
        assert_eq!((native.width, native.height, native.natural), (1024, 768, (1024, 768)));
        assert_eq!((native.width, native.height), (reference.width, reference.height));
        assert_eq!(native.pixels.len(), reference.pixels.len());

        let direct = || {
            let started = Instant::now();
            for _ in 0..5 {
                black_box(super::jpeg(black_box(&bytes), |size| size).unwrap());
            }
            started.elapsed().as_secs_f64() * 200.0
        };
        let fallback = || {
            let started = Instant::now();
            for _ in 0..5 {
                black_box(super::any(black_box(&bytes), |size| size).unwrap());
            }
            started.elapsed().as_secs_f64() * 200.0
        };

        direct();
        fallback();
        let mut direct_samples = Vec::new();
        let mut fallback_samples = Vec::new();
        for round in 0..8 {
            if round % 2 == 0 {
                fallback_samples.push(fallback());
                direct_samples.push(direct());
            } else {
                direct_samples.push(direct());
                fallback_samples.push(fallback());
            }
        }
        direct_samples.sort_by(f64::total_cmp);
        fallback_samples.sort_by(f64::total_cmp);
        let fast_ms = direct_samples[direct_samples.len() / 2];
        let original_ms = fallback_samples[fallback_samples.len() / 2];
        println!(
            "IMAGE_PERF format=jpeg pixels=786432 fallback_median_ms={original_ms:.3} native_median_ms={fast_ms:.3} ratio={:.3}",
            original_ms / fast_ms,
        );
    }

    #[test]
    fn tall_images_respect_the_pixel_budget() {
        assert_eq!(fit((1000, 5000), 1600, 1600.0 * 1600.0), (716, 3578));
        assert_eq!(fit((800, 600), 1600, 1600.0 * 1600.0), (800, 600));
    }
}
