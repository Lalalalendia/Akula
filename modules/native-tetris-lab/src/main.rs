use akula_native_tetris_lab::{
    raster, Action, Game, LOGICAL_HEIGHT, LOGICAL_WIDTH,
};
use std::{
    env,
    fs::File,
    io::{BufWriter, Write},
    time::Instant,
};

fn p95(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * 0.95).ceil() as usize]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frames: usize = env::args().nth(1).unwrap_or_else(|| "1200".into()).parse()?;
    if !(1..=10_000).contains(&frames) {
        return Err("frames must be in 1..=10000".into());
    }
    let filename = env::args().nth(2).unwrap_or_else(|| "tetris-last-frame.ppm".into());
    let mut game = Game::new(2026);
    let mut pixels = vec![0u32; (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize];
    let mut sim = Vec::with_capacity(frames);
    let mut draw = Vec::with_capacity(frames);
    let start = Instant::now();
    for frame in 0..frames {
        let t = Instant::now();
        if frame % 60 == 5 { game.press(Action::RotateClockwise); }
        if frame % 60 == 12 { game.press(Action::Left); }
        if frame % 60 == 26 { game.press(Action::Right); }
        if frame % 60 == 42 { game.press(Action::HardDrop); }
        game.tick();
        game.tick(); // 60 rendered frames/sec in simulated time, 120 ticks/sec.
        sim.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        raster(&game, &mut pixels, LOGICAL_WIDTH as usize, LOGICAL_WIDTH, LOGICAL_HEIGHT)
            .map_err(|e| format!("raster: {e:?}"))?;
        draw.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let total_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut file = BufWriter::new(File::create(&filename)?);
    write!(file, "P6\n{LOGICAL_WIDTH} {LOGICAL_HEIGHT}\n255\n")?;
    for color in pixels {
        file.write_all(&[
            (color >> 16) as u8,
            (color >> 8) as u8,
            color as u8,
        ])?;
    }
    file.flush()?;
    println!(
        "AKULA_TETRIS_LAB frames={frames} ticks={} lines={} score={} phase={:?} hash={:016x} sim_p95_ms={:.5} raster_p95_ms={:.5} total_ms={total_ms:.2} output_present=NOT_TESTED ppm={filename}",
        game.ticks, game.lines, game.score, game.phase, game.fingerprint(),
        p95(&mut sim), p95(&mut draw)
    );
    Ok(())
}
