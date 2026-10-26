use dcmfx::{core::*, p10::*, pixel_data::*};
use std::{env,time::Instant};
fn main(){
    let path=env::args().nth(1).expect("path");
    let start=Instant::now();
    let result=(|| -> Result<(usize,usize,usize),String>{
        let ds=DataSet::read_p10_file(&path,None).map_err(|e| format!("{e:?}"))?;
        let parse_us=start.elapsed().as_micros() as usize;
        let frames=ds.get_pixel_data_frames().map_err(|e| format!("{e:?}"))?;
        let renderer=PixelDataRenderer::from_data_set(&ds).map_err(|e| format!("{e:?}"))?;
        let mut count=0; let mut bytes=0;
        for mut frame in frames {
            let image=renderer.render_frame(&mut frame,None).map_err(|e| format!("{e:?}"))?;
            bytes+=image.as_raw().len();
            count+=1;
        }
        Ok((parse_us,bytes,count))
    })();
    match result {
        Ok((parse_us,bytes,frames))=>println!("RESULT library=dcmfx status=decoded parse_us={} total_us={} pixel_bytes={} frames={}",parse_us,start.elapsed().as_micros(),bytes,frames),
        Err(err)=>{eprintln!("RESULT library=dcmfx status=error total_us={} error={:?}",start.elapsed().as_micros(),err);std::process::exit(2)}
    }
}
