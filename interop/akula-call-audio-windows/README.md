# AK-CALL-02 — Windows sound-device laboratory

This is a manually launched, **non-shipping** console diagnostic. It is not wired to Akula Core, Native, startup, Iroh, Opus, recording, or calls.

## Safe behavior

- Lists input/output device names without opening microphone capture.
- Opens the default *output* device only after the user types the exact confirmation `PLAY`.
- Generates a quiet 440 Hz local tone for at most one second, then closes the output stream after two seconds. No file, network, microphone capture, or recording.
- The sample format must be f32; unsupported devices produce an error instead of being silently reconfigured.
- No automatic audio capture. Microphone opening and lifetime/permission logic are deferred until consent, mute/revoke and OS permission behavior can be tested properly.
- No guarantee for Windows 7; hardware compatibility must be verified separately.

## Manual test (Windows)

```powershell
cargo run --manifest-path interop/akula-call-audio-windows/Cargo.toml
```

Enter anything except `PLAY` to exit without opening a device.

## Engineering acceptance still open

- Validate device unplug during playback, device busy/denied permission and orderly stream stop.
- Implement opt-in microphone permission and capture with explicit second consent; ensure no capture before accepted call and stop on mute/revoke/disconnect.
- Connect bounded audio queues to the tested Opus + authenticated call, then measure physical audio delays.
- Verify Windows 7 separately, including cpal/backend support and minimum executable requirements.
