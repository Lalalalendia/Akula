use dcmfx::{core::*, p10::*, pixel_data::*};
use sha2::{Digest, Sha256};
use std::{env, time::Instant};

fn hash_monochrome(sha: &mut Sha256, data: &MonochromeImageData) {
    match data {
        MonochromeImageData::Bitmap { data, .. } => sha.update(data),
        MonochromeImageData::U8(data) => sha.update(data),
        MonochromeImageData::I8(data) => {
            for v in data { sha.update([*v as u8]); }
        }
        MonochromeImageData::U16(data) => {
            for v in data { sha.update(v.to_le_bytes()); }
        }
        MonochromeImageData::I16(data) => {
            for v in data { sha.update(v.to_le_bytes()); }
        }
        MonochromeImageData::U32(data) => {
            for v in data { sha.update(v.to_le_bytes()); }
        }
        MonochromeImageData::I32(data) => {
            for v in data { sha.update(v.to_le_bytes()); }
        }
    }
}
fn main() {
    let path = env::args().nth(1).expect("path");
    let start = Instant::now();
    let result = (|| -> Result<(u128, usize, usize, String), String> {
        let ds = DataSet::read_p10_file(&path, None).map_err(|e| format!("{e:?}"))?;
        let parse_us = start.elapsed().as_micros();
        let frames = ds.get_pixel_data_frames().map_err(|e| format!("{e:?}"))?;
        let renderer = PixelDataRenderer::from_data_set(&ds).map_err(|e| format!("{e:?}"))?;
        let mut count = 0;
        let mut shown_bytes = 0;
        let mut sha = Sha256::new();
        for mut frame in frames {
            if renderer.image_pixel_module.is_monochrome() {
                let mono = renderer.decode_monochrome_frame(&mut frame)
                    .map_err(|e| format!("{e:?}"))?;
                hash_monochrome(&mut sha, mono.data());
                let rgb = renderer.render_monochrome_image(&mono, None);
                shown_bytes += rgb.as_raw().len();
            } else {
                let color = renderer.decode_color_frame(&mut frame)
                    .map_err(|e| format!("{e:?}"))?;
                let rgb = color.into_rgb_u8_image();
                sha.update(rgb.as_raw());
                shown_bytes += rgb.as_raw().len();
            }
            count += 1;
        }
        Ok((parse_us, shown_bytes, count, format!("{:x}", sha.finalize())))
    })();
    match result {
        Ok((parse_us, bytes, frames, raw_sha)) => println!(
            "RESULT library=dcmfx status=decoded parse_us={} total_us={} pixel_bytes={} frames={} raw_sha256={}",
            parse_us, start.elapsed().as_micros(), bytes, frames, raw_sha
        ),
        Err(err) => {
            eprintln!("RESULT library=dcmfx status=error total_us={} error={:?}",
                      start.elapsed().as_micros(), err);
            std::process::exit(2);
        }
    }
}
