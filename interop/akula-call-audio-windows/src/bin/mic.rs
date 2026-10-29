//! Optional Windows microphone diagnostic. Samples are discarded, not recorded.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StopReason {
    Mute,
    Revoke,
    Disconnect,
    Stop,
    UnknownCommand,
    InputClosed,
    DeviceError,
    Timeout,
}

#[cfg(windows)]
fn approved(input: &str) -> bool {
    input.trim_end_matches(['\r', '\n']) == "CAPTURE"
}

#[cfg(windows)]
fn stop_reason(input: &str) -> StopReason {
    match input.trim_end_matches(['\r', '\n']) {
        "MUTE" => StopReason::Mute,
        "REVOKE" => StopReason::Revoke,
        "DISCONNECT" => StopReason::Disconnect,
        "STOP" => StopReason::Stop,
        _ => StopReason::UnknownCommand,
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This diagnostic requires Windows.");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::{
        io::{self, Write},
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        time::{Duration, Instant},
    };

    println!("Microphone diagnostic: samples are discarded, never saved or sent.");
    println!("Not connected to any Akula call. OS device permissions still apply.");
    print!("Type CAPTURE to authorize up to five seconds of local input: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 || !approved(&answer) {
        println!("Microphone not opened.");
        return Ok(());
    }

    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No default microphone")?;
    let config = device.default_input_config()?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err("Only f32 microphone input is supported".into());
    }

    let device_failed = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&device_failed);
    let stream = device.build_input_stream(
        &config.config(),
        |_samples: &[f32], _| {},
        move |_error| {
            signal.store(true, Ordering::Release);
        },
        None,
    )?;
    stream.play()?;
    println!("Active. Type STOP, MUTE, REVOKE, or DISCONNECT to close.");
    println!("DISCONNECT is a local test command, not a network signal.");

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut command = String::new();
        let received = match io::stdin().read_line(&mut command) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(command),
        };
        let _ = tx.send(received);
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let reason = loop {
        if device_failed.load(Ordering::Acquire) {
            break StopReason::DeviceError;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break StopReason::Timeout;
        }
        match rx.recv_timeout(remaining.min(Duration::from_millis(25))) {
            Ok(Some(command)) => break stop_reason(&command),
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                break StopReason::InputClosed;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };

    let _ = stream.pause();
    drop(stream);
    println!("Microphone closed ({reason:?}). No samples retained.");
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn capture_requires_explicit_exact_confirmation() {
        assert!(approved("CAPTURE\r\n"));
        for invalid in [
            "",
            "capture",
            " CAPTURE",
            "CAPTURE ",
            "CAPTURE_MORE",
            "PLAY",
        ] {
            assert!(!approved(invalid));
        }
    }

    #[test]
    fn stop_commands_cannot_reauthorize_capture() {
        assert_eq!(stop_reason("MUTE\n"), StopReason::Mute);
        assert_eq!(stop_reason("REVOKE\n"), StopReason::Revoke);
        assert_eq!(stop_reason("DISCONNECT\n"), StopReason::Disconnect);
        assert_eq!(stop_reason("STOP\r\n"), StopReason::Stop);
        assert_eq!(stop_reason("UNMUTE\n"), StopReason::UnknownCommand);
        for command in ["MUTE", "REVOKE", "DISCONNECT", "STOP"] {
            assert!(!approved(command));
        }
    }
}
