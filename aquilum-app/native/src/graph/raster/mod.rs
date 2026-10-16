mod shapes;
#[cfg(test)]
mod tests;

use std::ops::Range;

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub color: Rgb,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Disc {
    pub center: [f32; 2],
    pub radius: f32,
    pub body: Rgb,
    pub rim: Rgb,
    pub inner: f32,
    pub alpha: f32,
}

pub struct Sprite {
    pub width: usize,
    pub height: usize,
    pub fill: Vec<u8>,
    pub halo: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct Stamp<'a> {
    pub sprite: &'a Sprite,
    pub x: i32,
    pub y: i32,
    pub fill: Rgb,
    pub halo: Rgb,
    pub alpha: f32,
}

pub const BAND: usize = 64;

#[inline]
fn blend(px: &mut [u8], color: Rgb, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    if alpha >= 1.0 {
        px[..3].copy_from_slice(&color);
        return;
    }
    let a = (alpha * 256.0) as i32;
    for i in 0..3 {
        let d = i32::from(px[i]);
        px[i] = (d + (((i32::from(color[i]) - d) * a) >> 8)) as u8;
    }
}

pub struct Band<'b> {
    pub rows: Range<usize>,
    pub width: usize,
    pixels: &'b mut [u8],
}

impl Band<'_> {
    #[inline]
    fn plot(&mut self, x: i32, y: i32, color: Rgb, alpha: f32) {
        if x < 0 || y < self.rows.start as i32 || y >= self.rows.end as i32 || x >= self.width as i32 {
            return;
        }
        let at = ((y as usize - self.rows.start) * self.width + x as usize) * 4;
        blend(&mut self.pixels[at..at + 4], color, alpha);
    }
}

pub fn bands(width: usize, height: usize, pixels: &mut [u8], draw: impl Fn(&mut Band<'_>) + Sync) {
    if width == 0 || height == 0 || pixels.len() < width * height * 4 {
        return;
    }
    use rayon::prelude::*;
    pixels[..width * height * 4].par_chunks_mut(BAND * width * 4).enumerate().for_each(|(index, chunk)| {
        let rows = index * BAND..((index + 1) * BAND).min(height);
        draw(&mut Band { rows, width, pixels: chunk });
    });
}

pub fn dilate(mask: &[u8], width: usize, height: usize) -> Vec<u8> {
    // Preserve the original zero-dimension contract: output length is mask.len().
    if width == 0 || height == 0 {
        return vec![0u8; mask.len()];
    }
    // A 3x3 maximum filter is separable: 1x3 horizontal then 3x1 vertical.
    // The extra temporary buffer trades one linear scan for fewer comparisons.
    let mut horizontal = vec![0u8; mask.len()];
    for y in 0..height {
        let row_start = y * width;
        let src = &mask[row_start..row_start + width];
        let dst = &mut horizontal[row_start..row_start + width];
        for x in 0..width {
            dst[x] = src[x]
                .max(src[x.saturating_sub(1)])
                .max(src[(x + 1).min(width - 1)]);
        }
    }

    let mut out = vec![0u8; mask.len()];
    for y in 0..height {
        let prev = y.saturating_sub(1) * width;
        let current = y * width;
        let next = (y + 1).min(height - 1) * width;
        for x in 0..width {
            out[current + x] = horizontal[prev + x]
                .max(horizontal[current + x])
                .max(horizontal[next + x]);
        }
    }
    out
}
