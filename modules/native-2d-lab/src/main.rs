use akula_native_2d_lab::{raster, Input, World, WIDTH, HEIGHT, DT};
use std::{env, fs::File, io::{BufWriter, Write}, time::Instant};
fn percentile(values: &mut [f64], ratio: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * ratio).ceil() as usize]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frames:usize = env::args().nth(1).unwrap_or_else(||"1200".into()).parse()?;
    if !(1..=100_000).contains(&frames) { return Err("frames outside 1..=100000".into()); }
    let mut world = World::new();
    let mut pixels = vec![0u32;WIDTH as usize * HEIGHT as usize];
    let mut simulation_ms=Vec::with_capacity(frames);
    let mut raster_ms=Vec::with_capacity(frames);
    let t0=Instant::now();
    let mut accumulator=0.0f32;
    for frame in 0..frames {
        let t=Instant::now();
        accumulator += 1.0/60.0;
        let mut ticks = 0;
        while accumulator >= DT && ticks < 4 {
            world.step(Input {
                start:frame==0 && ticks==0,
                left:frame%120<40,
                right:frame%120>=60 && frame%120<100,
            });
            accumulator -= DT;
            ticks+=1;
        }
        simulation_ms.push(t.elapsed().as_secs_f64()*1000.0);
        let t=Instant::now();
        raster(&world,&mut pixels,WIDTH as usize,WIDTH,HEIGHT)
            .map_err(|err|format!("raster: {err:?}"))?;
        raster_ms.push(t.elapsed().as_secs_f64()*1000.0);
    }
    let total_ms=t0.elapsed().as_secs_f64()*1000.0;
    let sim_p95=percentile(&mut simulation_ms,0.95);
    let raster_p95=percentile(&mut raster_ms,0.95);
    let path=env::args().nth(2).unwrap_or_else(||"game2d-last-frame.ppm".into());
    let mut out=BufWriter::new(File::create(&path)?);
    write!(out,"P6\n{WIDTH} {HEIGHT}\n255\n")?;
    for argb in pixels {out.write_all(&[((argb>>16)&255) as u8,((argb>>8)&255) as u8,(argb&255) as u8])?;}
    out.flush()?;
    println!("AKULA-NATIVE-2D frames={frames} ticks={} hash={:016x} sim_p95_ms={sim_p95:.5} raster_p95_ms={raster_p95:.5} total_ms={total_ms:.2} output_present=NOT_TESTED ppm={path}",world.ticks,world.fingerprint());
    Ok(())
}
