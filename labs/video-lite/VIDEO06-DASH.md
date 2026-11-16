# VIDEO-06 — Fragmented DASH/fMP4 acceptance in pure Rust

Purpose: eliminate the biggest untested assumption after VIDEO-05 (#966):
a normal MP4 fixture does **not** prove that YouTube-style separate fragmented
DASH video and audio can be remuxed without FFmpeg in Akula.

The initial experiment proved that the 2023-era `mp4 = 0.14.0` reader
cannot handle fragmented tracks when `trun` omits per-sample sizes and
`tfhd`/`trex` provides defaults (`EntryInTrunNotFound`). The corrected CLI uses
**`mp4-track = 0.1.0`**, a bounded pure-Rust demuxer/writer, instead.
The lab adds a small CLI
(`cargo run --manifest-path labs/video-lite/Cargo.toml --bin dash -- prove-pair
<video.mp4> <audio.m4a> <result.mp4>`).

## Test contract

- Generate video-only and audio-only *fragmented* sources from the original,
  self-created two-second H.264/AAC clip, repeated to around 60 seconds.
- Use the CI runner's FFmpeg only as a **test fixture generator**, never as an
  Akula runtime dependency. Never download a real video or visit YouTube.
- Verify that each generated MP4 has multiple top-level `moof` boxes.
- Parse separate video and audio fMP4 inputs using the Rust `mp4` crate,
  preserve codec configuration, sample bytes, decode-time ticks, durations,
  composition offsets and sync flags. Interleave by timestamp using integer
  timebase arithmetic without needing to load full input files into RAM.
- Reopen the progressive MP4 output and compare each encoded sample against
  its source. Run the same acceptance on both Linux and Windows.
- Independently validate both codecs, around 60-second duration, and successful
  full H.264 + AAC decoding through a reference FFmpeg in Linux CI; the
  reference decoder does not participate in Rust multiplexing.
- No installable app, updater, cloud sync, vault data or third-party bundles
  are produced.

## Release blockers after passing

1. Real authorized YouTube download (and ongoing upstream extractor changes)
   is not covered; use only permitted content.
2. Other DASH variants, fragmented topologies (including multiple tracks per
   `traf`, `sidx`, discontinuities), WebM, subtitles, DRM, multi-hour scale,
   crash/power-failure recovery and media compatibility need additional tests.
3. Native integration with VIDEO-03 downloader is not included here. It must
   be landed behind explicit-user-action and local-library-only invariants.
4. Windows 7 actual VM and official yt-dlp Win10+ executable mismatch remain
   separate blockers.

The Rust reader is configured with explicit limits: 8 MiB movie metadata,
2 MiB per fragment box, 8 MiB per sample, 20,000 fragments, 400,000 samples
per track, and nesting depth 20. These lab values are not a substitute for
threat-model review, fuzzing, or testing the size limits on lengthy media.

This is a proof of format support, not acceptance of arbitrary input files.
