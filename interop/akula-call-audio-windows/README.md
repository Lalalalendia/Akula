# AK-CALL-02 — Windows audio device laboratory

Standalone **non-shipping** Windows diagnostics. No Akula runtime integration,
Iroh/QUIC, Opus, automatic startup, file writes or network access.
These are not working calls or release candidates.

## Manual output diagnostic

The default executable lists device names but never opens a microphone.
Only the exact `PLAY` confirmation opens default output. It plays a quiet
440 Hz tone for at most one second and closes the output within two seconds.

```powershell
cargo run --manifest-path interop/akula-call-audio-windows/Cargo.toml --bin akula-call-audio-windows
```

## Manual microphone diagnostic

Use a separate console command to run the `mic` binary. The exact word
`CAPTURE` is required before opening the default input device. Windows
privacy settings remain authoritative. Audio samples are immediately
discarded by the device callback, without storage, processing or sending.

```powershell
cargo run --manifest-path interop/akula-call-audio-windows/Cargo.toml --bin mic
```

During capture, `STOP`, `MUTE`, `REVOKE`, or `DISCONNECT` closes the stream.
Unknown input also ends capture. Input failure, device callback error and
the five-second deadline terminate the probe. These commands are **local
simulations**, NOT actual network-disconnect or remote-revoke notifications.
The program uses a separate console-input thread so it can time out even
if the operator types nothing. Both probes accept only default f32 formats.

## Verification and limitations

Windows CI checks formatting, compilation, pure logic tests and Clippy.
It cannot prove audio hardware operation, unplug behavior, Windows 7,
permission dialogs, latency or physical microphone privacy.
No audio devices are opened by automated tests.

**Keep Draft/HOLD.** Before any real call: connect accepted-call and
OS-authorized device lifecycles, enforce revoke and disconnect at the host,
build bounded PCM queues, integrate Opus/QUIC and test two physical PCs.
