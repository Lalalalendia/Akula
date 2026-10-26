use dicom_object::open_file;
use dicom_pixeldata::PixelDecoder;
use sha2::{Digest, Sha256};
use std::{env, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let path = args.next().expect("DICOM path");
    let index: u32 = args.next().expect("zero-based frame index").parse()?;
    let start = Instant::now();
    let obj = open_file(path)?;
    let pixels = obj.decode_pixel_data_frame(index)?;
    let raw = pixels.data();
    println!(
        "SLICE library=dicom-rs frame={} frames={} pixel_bytes={} raw_sha256={} total_us={}",
        index, pixels.number_of_frames(), raw.len(), format!("{:x}", Sha256::digest(raw)),
        start.elapsed().as_micros()
    );
    Ok(())
}
