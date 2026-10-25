use std::io::Read;
use std::path::PathBuf;

use akula_flash_port::scene::{emit_svg_frame, parse_scene};
use akula_flash_port::{MAX_SWF_BYTES, emit_rust, parse_swf};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let first = args
        .next()
        .ok_or("usage: akula-flash-port [--scene-svg FRAME] <input.swf> <output>")?;
    let (scene_frame, input_arg) = if first == "--scene-svg" {
        let index = args
            .next()
            .ok_or("missing frame index")?
            .into_string()
            .map_err(|_| "frame index must be valid UTF-8")?
            .parse::<usize>()?;
        (Some(index), args.next())
    } else {
        (None, Some(first))
    };
    let input = input_arg
        .map(PathBuf::from)
        .ok_or("usage: akula-flash-port <input.swf> <output.rs>")?;
    let output = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: akula-flash-port <input.swf> <output.rs>")?;
    if args.next().is_some() {
        return Err("too many arguments".into());
    }

    let file = std::fs::File::open(&input)?;
    let mut bytes = Vec::new();
    file.take((MAX_SWF_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SWF_BYTES {
        return Err("SWF exceeds the 8 MiB P0 limit".into());
    }

    if let Some(index) = scene_frame {
        let scene = parse_scene(&bytes)?;
        let svg = emit_svg_frame(&scene, index)?;
        std::fs::write(&output, svg)?;
        eprintln!(
            "SWF scene: {} frames; exported frame {} to {}",
            scene.frames.len(),
            index,
            output.display()
        );
        return Ok(());
    }
    let movie = parse_swf(&bytes)?;
    let code = emit_rust(&movie)?;
    std::fs::write(&output, code)?;
    eprintln!(
        "SWF{}: {} AVM1 actions, {} frame(s); Rust source written to {}",
        movie.version,
        movie.actions.len(),
        movie.frame_count,
        output.display()
    );
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("akula-flash-port: {err}");
        std::process::exit(1);
    }
}
