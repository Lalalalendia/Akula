# VIDEO-07 — Rust mux-pair command for a future Akula downloader

**Prototype only; not connected to the installed Akula application.**

The prior VIDEO-06 command `prove-pair` was deliberately strict: it
required three or more fragmented MP4 boxes, reread and compared every
compressed sample, and rejected any other case. Those requirements were
appropriate for CI but not for short, valid media.

VIDEO-07 introduces the `mux-pair` command:
```text
dash mux-pair <H264-only.mp4> <AAC-only.m4a> <new-output.mp4>
```

The command accepts progressive MP4 and short/single-fragment fMP4 as well
as longer DASH-style fMP4. Both sources must contain one track, with H.264
and AAC respectively, and must start with DTS 0. Other codecs, multiple
tracks and nonzero starts are explicitly rejected rather than silently
misaligned. The result is generated through the bounded Rust `mp4-track`
parser and writer with no FFmpeg involved.

Safety properties: output uses `create_new` (no replacement), is flushed
to disk before success, and is removed if any step fails; oversized MP4
metadata and samples and excessive fragment counts are rejected. A future
Akula integration must additionally ensure input/output paths are inside a
private per-device staging directory outside all synced workspaces; this
standalone lab does not know vault roots.

Acceptance in CI:
- Generate single-fragment and progressive video/audio-only H.264/AAC
  inputs from an original synthetic fixture.
- Windows and Linux run both `mux-pair` modes and the strict
  multi-fragment `prove-pair` unchanged.
- Reject attempts to overwrite successful output and verify its hash
  remains unchanged.
- Reject a video/audio codec swap and leave no failed-output file.
- Independently decode both output media types with FFmpeg in Linux CI.

Remaining blockers for real Akula: integrate separate explicit yt-dlp
video-only and audio-only downloads into device-local staging, call the
validated Rust muxer in that staging directory, verify the result with
native playback, publish atomically, keep QuickJS and yt-dlp distribution
pinned, and test an authorized YouTube video end-to-end. The official
yt-dlp.exe used in the slim-lab remains unsupported on Windows 7.
