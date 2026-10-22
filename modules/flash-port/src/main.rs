use std::io::Read;
use std::path::PathBuf;

use akula_flash_port::{MAX_SWF_BYTES, emit_rust, parse_swf};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
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
