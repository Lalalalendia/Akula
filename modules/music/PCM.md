# MUSIC-04a — bounded PCM reference buffer

Independent WIP above MUSIC-03, not shipping or a real device benchmark.
The fixed single-owner PCM16 ring allocates once; push/pull do not allocate.
Capacity 1–2,000 ms, at most 4 MiB of sample payload, 8–384 kHz and 1–8 channels.
Initial target for A/B is 250 ms, not an accepted production configuration.
Partial push preserves unread data, pull fills underruns with silence.
A discontinuity resets queue and fences old tickets; same-session pause/resume
may retag without discarding buffered samples. External owner must hold audio
on pause/seek, cancel old I/O and drain/stop actual output before transition.
Delivered PCM frames are not confirmed hardware-rendered frames; silence
padding is excluded from reported audio frames. No real-time or thread-safe
claim. Future callback-safe SPSC handoff needs separate proof.

Reference candidates: existing native Windows Media Foundation PCM16 + WASAPI
shared (requested 200 ms buffer), Linux GStreamer; compare Rodio/Symphonia/CPAL
as a separate test, not yet a dependency. Real-device tests must measure
startup, seek, underruns, CPU/RSS and concurrent editor/ASR load; no synthetic
ring result alone selects a backend. Seven Rust tests are staged, unexecuted.
No new dependencies, no new workflow, no installer/release change.
