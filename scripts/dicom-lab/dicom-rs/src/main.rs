use dicom_object::open_file;
use dicom_pixeldata::PixelDecoder;
use std::{env, time::Instant};
fn main() {
    let path=env::args().nth(1).expect("path");
    let start=Instant::now();
    let result=(|| -> Result<(usize,usize,usize),Box<dyn std::error::Error>> {
        let obj=open_file(&path)?;
        let parsed=start.elapsed().as_micros();
        let pixels=obj.decode_pixel_data()?;
        Ok((parsed as usize,pixels.data().len(),pixels.number_of_frames() as usize))
    })();
    match result {
       Ok((parse_us,bytes,frames))=>println!("RESULT library=dicom-rs status=decoded parse_us={} total_us={} pixel_bytes={} frames={}",parse_us,start.elapsed().as_micros(),bytes,frames),
       Err(err)=>{eprintln!("RESULT library=dicom-rs status=error total_us={} error={:?}",start.elapsed().as_micros(),err); std::process::exit(2)}
    }
}
