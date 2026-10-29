#[cfg(not(windows))]
fn main() { eprintln!("Windows only"); }
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::{io::{self, Write}, time::{Duration, Instant}};
    println!("Microphone test; sound is discarded and never saved or sent.");
    print!("Type CAPTURE to authorize a five-second test: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim_end_matches(['\r', '\n']) != "CAPTURE" { return Ok(()); }
    let device = cpal::default_host().default_input_device().ok_or("No microphone")?;
    let config = device.default_input_config()?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err("Unsupported input format".into());
    }
    let stream = device.build_input_stream(
        &config.config(), |_samples: &[f32], _| {}, |_error| {}, None
    )?;
    stream.play()?;
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end { std::thread::sleep(Duration::from_millis(25)); }
    drop(stream);
    println!("Microphone closed.");
    Ok(())
}
