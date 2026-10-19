//! Standalone comparison using Akula's actual decoder implementation.
//! Three commands: generate PPM, decode via Akula, compare two RGBA buffers.
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

#[path = "../../../aquilum-app/native/src/images/decode.rs"]
mod akula_decoder;

const MAX_WIDTH: u32 = 1600;
const MAX_PIXELS: f64 = 1600.0 * 1600.0;
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn generate(path: &Path, width: usize, height: usize) -> Result<()> {
    if !(1..=20000).contains(&width) || !(1..=20000).contains(&height) {
        return Err("fixture width/height must be 1..=20000".into());
    }
    let mut dest = BufWriter::new(File::create(path)?);
    write!(dest, "P6\n{} {}\n255\n", width, height)?;
    let mut row = vec![0u8; width.checked_mul(3).ok_or("row too large")?];
    for y in 0..height {
        for x in 0..width {
            let i = x * 3;
            let z = ((x as u64).wrapping_mul(1_664_525)
                ^ (y as u64).wrapping_mul(1_013_904_223)) as u32;
            row[i] = ((x * 13 + y * 3) & 255) as u8;
            row[i+1] = ((y * 11 + (x / 7) * 19) & 255) as u8;
            row[i+2] = (((z >> 15) as usize ^ (x / 8) ^ (y / 8)) & 255) as u8;
        }
        dest.write_all(&row)?;
    }
    dest.flush()?;
    Ok(())
}

fn decode_akula(src: &Path, out: &Path) -> Result<()> {
    let start = Instant::now();
    let compressed = std::fs::read(src)?;
    let result = akula_decoder::decode(&compressed, MAX_WIDTH, MAX_PIXELS)
        .ok_or("Akula decoder rejected the fixture")?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    std::fs::write(out, &result.pixels)?;
    println!("RESULT mode=akula original={}x{} shown={}x{} compressed={} rgba={} read_decode_ms={:.3}",
        result.natural.0, result.natural.1, result.width, result.height,
        compressed.len(), result.pixels.len(), ms);
    Ok(())
}
fn compare(a: &Path, b: &Path) -> Result<()> {
    let first = std::fs::read(a)?;
    let second = std::fs::read(b)?;
    if first.is_empty() || first.len() != second.len() || first.len() % 4 != 0 {
        return Err(format!("invalid/different RGBA buffers: {} vs {}",first.len(),second.len()).into());
    }
    let mut sum_abs = 0u64;
    let mut sum_squares = 0u64;
    let mut max_diff = 0u8;
    let mut alpha_mismatch = 0usize;
    for (i, (&left,&right)) in first.iter().zip(&second).enumerate() {
        let delta = left.abs_diff(right);
        if i % 4 == 3 {
            if delta != 0 { alpha_mismatch += 1; }
        } else {
            sum_abs += delta as u64;
            sum_squares += u64::from(delta) * u64::from(delta);
            max_diff = max_diff.max(delta);
        }
    }
    let n = (first.len()/4*3) as f64;
    let rmse = (sum_squares as f64/n).sqrt();
    let psnr = if rmse == 0.0 { f64::INFINITY } else { 20.0 * (255.0/rmse).log10() };
    println!("COMPARE A={} B={} rgb_mae={:.5} rgb_rmse={:.5} rgb_psnr_db={:.4} max_diff={} alpha_mismatches={}",
        a.display(),b.display(),sum_abs as f64/n,rmse,psnr,max_diff,alpha_mismatch);
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    match args.as_slice() {
        [_, cmd, out, w, h] if cmd == "generate" => generate(Path::new(out),w.parse()?,h.parse()?),
        [_, cmd, src, out] if cmd == "akula" => decode_akula(Path::new(src),Path::new(out)),
        [_, cmd, a, b] if cmd == "compare" => compare(Path::new(a),Path::new(b)),
        _ => Err("usage: akula-image-bench generate <out.ppm> <width> <height> | akula <src.jpg> <out.rgba> | compare <a.rgba> <b.rgba>".into()),
    }
}
