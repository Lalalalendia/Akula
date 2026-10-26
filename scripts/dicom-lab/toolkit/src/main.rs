use dicom_toolkit_data::{FileFormat, PixelData, Value};
use dicom_toolkit_dict::tags;
use dicom_toolkit_image::DicomImage;
use sha2::{Digest, Sha256};
use std::{env, time::Instant};

fn main() {
    let path = env::args().nth(1).expect("path");
    let start = Instant::now();
    let result = (|| -> Result<(u128, usize, usize, String), Box<dyn std::error::Error>> {
        let mut ff = FileFormat::open(&path)?;
        let parse_us = start.elapsed().as_micros();
        let ts = ff.meta.transfer_syntax_uid.clone();
        if !["1.2.840.10008.1.2", "1.2.840.10008.1.2.1"].contains(&ts.as_str()) {
            let rows = ff.dataset.get_u16(tags::ROWS).ok_or("missing rows")?;
            let cols = ff.dataset.get_u16(tags::COLUMNS).ok_or("missing columns")?;
            let bits = ff.dataset.get_u16(tags::BITS_ALLOCATED).ok_or("missing bits")?;
            let samples = ff.dataset.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1);
            let elem = ff.dataset.find_element(tags::PIXEL_DATA)?;
            let Value::PixelData(pixel) = &elem.value else { return Err("not encapsulated".into()) };
            let compressed = pixel.encapsulated_frames(1)?;
            let raw = dicom_toolkit_codec::decode_pixel_data(
                &ts, &compressed[0], rows, cols, bits, samples
            )?;
            let target = ff.dataset.get_mut(tags::PIXEL_DATA).ok_or("missing pixel element")?;
            target.value = Value::PixelData(PixelData::Native { bytes: raw });
        }
        let image = DicomImage::from_dataset(&ff.dataset)?;
        let mut shown_bytes = 0;
        let mut sha = Sha256::new();
        for f in 0..image.frames {
            sha.update(image.frame_bytes(f)?);
            shown_bytes += image.frame_u8(f)?.len();
        }
        Ok((parse_us, shown_bytes, image.frames as usize, format!("{:x}", sha.finalize())))
    })();
    match result {
        Ok((parse_us, bytes, frames, raw_sha)) => println!(
            "RESULT library=toolkit status=decoded parse_us={} total_us={} pixel_bytes={} frames={} raw_sha256={}",
            parse_us, start.elapsed().as_micros(), bytes, frames, raw_sha
        ),
        Err(err) => {
            eprintln!("RESULT library=toolkit status=error total_us={} error={:?}",
                      start.elapsed().as_micros(), err);
            std::process::exit(2);
        }
    }
}
