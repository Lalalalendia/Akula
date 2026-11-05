//! Manually started local Windows audio test; never captures user audio.
//! This separate binary has no Akula runtime, storage or network connection.
#[cfg(windows)]
fn operator_approved(answer: &str) -> bool {
    // Accept exactly PLAY, allowing only the line terminator from stdin.
    answer.trim_end_matches(['\r', '\n']) == "PLAY"
}

#[cfg(windows)]
fn tone_sample(seconds: f32, enabled: bool) -> f32 {
    if enabled && (0.0..1.0).contains(&seconds) {
        (seconds * 440.0 * std::f32::consts::TAU).sin() * 0.03
    } else {
        0.0
    }
}

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
    println!("Input devices (enumerated only, never opened):");
    for device in host.input_devices()? {
        println!("  {}", device.name()?);
    }
    print!("Type PLAY to hear a short test tone (otherwise exit): ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !operator_approved(&answer) {
        println!("No sound device opened.");
        return Ok(());
    }

    let device = host
        .default_output_device()
        .ok_or("No default output device")?;
    let config = device.default_output_config()?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err("This laboratory requires a floating-point default output format".into());
    }
    let sample_rate = config.sample_rate().0 as f32;
    let channels = usize::from(config.channels());
    let enabled = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&enabled);
    let mut tick = 0_u64;
    let on_error = |err| eprintln!("Audio stream stopped with error: {err}");
    let stream = device.build_output_stream(
        &config.config(),
        move |data: &mut [f32], _| {
            for frame in data.chunks_mut(channels) {
                let seconds = tick as f32 / sample_rate;
                let sample = tone_sample(seconds, flag.load(Ordering::Relaxed));
                for item in frame {
                    *item = sample;
                }
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

#[cfg(all(test, windows))]
mod tests {
    use super::{operator_approved, tone_sample};

    #[test]
    fn no_device_access_without_exact_confirmation() {
        assert!(operator_approved("PLAY\r\n"));
        for attempt in ["", "play\n", " PLAY\n", "PLAY \n", "PLAYBACK\n", "yes\n"] {
            assert!(
                !operator_approved(attempt),
                "unexpected confirmation: {attempt:?}"
            );
        }
    }

    #[test]
    fn tone_is_bounded_and_silent_after_one_second_or_mute() {
        for millis in 0..1000 {
            assert!(tone_sample(millis as f32 / 1000.0, true).abs() <= 0.030_001);
            assert_eq!(tone_sample(millis as f32 / 1000.0, false), 0.0);
        }
        assert_eq!(tone_sample(1.0, true), 0.0);
        assert_eq!(tone_sample(5.0, true), 0.0);
        assert_eq!(tone_sample(-1.0, true), 0.0);
    }
}
