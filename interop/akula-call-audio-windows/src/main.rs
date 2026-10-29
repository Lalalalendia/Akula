//! Explicitly launched Windows-only audio device experiment.
//! No file persistence, network calls, or automatic capture.
#[cfg(not(windows))]
fn main() {
    eprintln!("This diagnostic requires Windows.");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::io::{self, Write};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::Duration;

    let host = cpal::default_host();
    println!("AK-CALL-02 local audio diagnostic (no recording or network)");
    println!("Output devices:");
    for device in host.output_devices()? {
        println!("  {}", device.name()?);
    }
    println!("Input devices:");
    for device in host.input_devices()? {
        println!("  {}", device.name()?);
    }
    print!("Type PLAY to hear a short test tone (otherwise exit): ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim() != "PLAY" {
        println!("No sound device opened.");
        return Ok(());
    }
    let device = host.default_output_device().ok_or("No default output device")?;
    let config = device.default_output_config()?;
    let rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    let enabled = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&enabled);
    let mut tick = 0_u64;
    let on_error = |err| eprintln!("Audio stream stopped with error: {err}");
    // Only f32 default output is supported by this initial conservative lab.
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err("This laboratory requires a floating-point default output format".into());
    }
    let stream = device.build_output_stream(
        &config.config(),
        move |data: &mut [f32], _| {
            for frame in data.chunks_mut(channels) {
                let t = tick as f32 / rate;
                let sample = if flag.load(Ordering::Relaxed) && t < 1.0 {
                    (t * 440.0 * std::f32::consts::TAU).sin() * 0.03
                } else {
                    0.0
                };
                for item in frame { *item = sample; }
                tick = tick.saturating_add(1);
            }
        },
        on_error,
        None,
    )?;
    stream.play()?;
    std::thread::sleep(Duration::from_secs(2));
    enabled.store(false, Ordering::SeqCst);
    drop(stream);
    println!("Output stopped. Nothing was recorded or transmitted.");
    Ok(())
}
