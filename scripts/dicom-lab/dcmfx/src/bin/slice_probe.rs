use dcmfx::{core::*, p10::*, pixel_data::*};
use sha2::{Digest, Sha256};
use std::{env, time::Instant};

fn run(path: &str, index: usize) -> Result<(), String> {
    let start = Instant::now();
    let dataset = DataSet::read_p10_file(path, None).map_err(|e| format!("{e:?}"))?;
    let frames = dataset.get_pixel_data_frames().map_err(|e| format!("{e:?}"))?;
    let mut frame = frames.into_iter().nth(index)
        .ok_or_else(|| format!("frame {} out of range", index))?;
    let renderer = PixelDataRenderer::from_data_set(&dataset)
        .map_err(|e| format!("{e:?}"))?;
    if !renderer.image_pixel_module.is_monochrome() {
        return Err("this synthetic performance probe expects monochrome".to_owned());
    }
    let image = renderer.decode_monochrome_frame(&mut frame)
        .map_err(|e| format!("{e:?}"))?;
    let mut hash = Sha256::new();
    let mut count = 0_usize;
    match image.data() {
        MonochromeImageData::I16(samples) => {
            for sample in samples { hash.update(sample.to_le_bytes()); count += 2; }
        }
        MonochromeImageData::U16(samples) => {
            for sample in samples { hash.update(sample.to_le_bytes()); count += 2; }
        }
        other => return Err(format!("unexpected pixel storage: {other:?}")),
    }
    println!(
        "SLICE library=dcmfx frame={} frames=1 pixel_bytes={} raw_sha256={} total_us={}",
        index, count, format!("{:x}", hash.finalize()), start.elapsed().as_micros()
    );
    Ok(())
}

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("DICOM path");
    let index: usize = args.next().expect("frame").parse().expect("numeric frame");
    if let Err(e) = run(&path, index) {
        eprintln!("SLICE_ERROR {e}");
        std::process::exit(1);
    }
}
