use dicom_toolkit_data::FileFormat;
use dicom_toolkit_image::DicomImage;
use sha2::{Digest, Sha256};
use std::{env, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let path = args.next().expect("DICOM path");
    let index: u32 = args.next().expect("zero-based frame index").parse()?;
    let start = Instant::now();
    let ff = FileFormat::open(path)?;
    let image = DicomImage::from_dataset(&ff.dataset)?;
    let raw = image.frame_bytes(index)?;
    println!(
        "SLICE library=toolkit frame={} frames=1 pixel_bytes={} raw_sha256={} total_us={}",
        index, raw.len(), format!("{:x}", Sha256::digest(raw)), start.elapsed().as_micros()
    );
    Ok(())
}
