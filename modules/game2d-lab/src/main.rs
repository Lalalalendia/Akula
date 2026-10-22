use akula_game2d_native_lab::{draw, Game, Input, DT, H, W};
use std::{env, fs::File, io::{BufWriter, Write}, time::Instant};
fn percentile(samples: &mut [f64], fraction: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * fraction).ceil() as usize]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frames: usize = env::args().nth(1).unwrap_or_else(||"1200".into()).parse()?;
    if frames == 0 || frames > 100_000 { return Err("frames must be 1..=100000".into()); }
    let mut game=Game::default();
    let mut pixels=vec![0u32;W*H];
    let mut simulation_ms=Vec::with_capacity(frames);
    let mut raster_ms=Vec::with_capacity(frames);
    let total_start=Instant::now();
    let mut accumulator=0.0f32;
    for n in 0..frames {
        let start=Instant::now();
        // 60 Hz simulated display with two 120 Hz logic ticks.
        accumulator += 1.0/60.0;
        let mut steps=0;
        while accumulator >= DT && steps < 4 {
            game.step(Input { left:n%120 < 40, right:n%120 >= 60 && n%120 < 100 });
            accumulator -= DT;
            steps+=1;
        }
        simulation_ms.push(start.elapsed().as_secs_f64()*1000.0);
        let start=Instant::now();
        draw(&game, &mut pixels, W,H)?;
        raster_ms.push(start.elapsed().as_secs_f64()*1000.0);
    }
    let total=total_start.elapsed().as_secs_f64()*1000.0;
    let sim_p95=percentile(&mut simulation_ms,0.95);
    let render_p95=percentile(&mut raster_ms,0.95);
    let path=env::args().nth(2).unwrap_or_else(||"game2d-last-frame.ppm".into());
    let mut out=BufWriter::new(File::create(&path)?);
    write!(out,"P6\n{W} {H}\n255\n")?;
    for argb in pixels { out.write_all(&[((argb>>16)&255) as u8,((argb>>8)&255) as u8,(argb&255) as u8])?; }
    out.flush()?;
    println!("GAME2D-LAB frames={frames} simulated_fps=60 ticks={} hash={:016x} sim_p95_ms={sim_p95:.5} raster_p95_ms={render_p95:.5} total_ms={total:.3} ppm={path} output_present=NOT_TESTED",game.ticks,game.state_hash());
    Ok(())
}
