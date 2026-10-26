use dicom_object::open_file;
use dicom_pixeldata::PixelDecoder;
use sha2::{Digest, Sha256};
use std::{env, time::Instant};

fn main() {
    let path = env::args().nth(1).expect("path");
    let start = Instant::now();
    let result = (|| -> Result<(u128, usize, usize, String), Box<dyn std::error::Error>> {
        let obj = open_file(&path)?;
        let parsed_us = start.elapsed().as_micros();
        let pixels = obj.decode_pixel_data()?;
        let raw = pixels.data();
        Ok((parsed_us, raw.len(), pixels.number_of_frames() as usize,
            format!("{:x}", Sha256::digest(raw))))
    })();
    match result {
        Ok((parse_us, bytes, frames, raw_sha)) => println!(
            "RESULT library=dicom-rs status=decoded parse_us={} total_us={} pixel_bytes={} frames={} raw_sha256={}",
            parse_us, start.elapsed().as_micros(), bytes, frames, raw_sha
        ),
        Err(err) => {
            eprintln!("RESULT library=dicom-rs status=error total_us={} error={:?}",
                      start.elapsed().as_micros(), err);
            std::process::exit(2);
        }
    }
}
