# VIDEO-05 — Pure Rust MP4 remux + QuickJS-NG size experiment

**Isolated non-release lab only.** It never downloads a YouTube video or
changes Akula's media library, sync boundaries, or public installer.

## Question

Can Akula replace the approximately 234 MB compressed / 385 MB executable
Win10 dependency trial (FFmpeg + Deno + yt-dlp, see PR #965) with a tiny
Windows JS runtime and an MP4-only Rust remuxer?

## Prototype components

- **Rust:** `mp4 = 0.14.0` (MIT). The separate lab CLI uses
  `Mp4Reader` / `Mp4Writer` to isolate H.264 and AAC into separate MP4/M4A
  files and then interleave those already-encoded samples by decode time into
  one new MP4. No decoding, transcoding or FFmpeg process.
- **JavaScript:** QuickJS-NG `v0.18.0`, official Windows x64 binary
  `qjs-windows-x86_64.exe` (1,873,152 bytes), pinned SHA-256
  `ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7`.
  QuickJS-NG is officially supported by yt-dlp as an optional external JS
  runtime; **only argument parsing and trivial JS evaluation are tested**.
- **Extractor:** `yt-dlp.exe` `2026.10.26`, pinned SHA-256
  `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d`.
  The standalone binary contains EJS scripts, not a working JS engine.
- **Fixture:** exact pre-existing original 2-second H.264/AAC test clip from
  VIDEO-03 (`b6e243a8e289e2f4a537a3a4312c2b503841efc6` Git blob).

## Acceptance

The Windows workflow records release binary size and validates that the
compressed H.264 and AAC samples, sample count and timing survive separate
track files and pure-Rust remux unchanged. The QuickJS job rejects tampered
bytes before execution, checks upstream pins, executes a harmless JS string,
parses the yt-dlp runtime option, and reports real compressed size.

## Important limitations

- A successful `mp4` crate test with **progressive MP4** does not prove
  support for every YouTube DASH/fMP4, CMAF, WebM or HLS input. Before a
  production replacement, add fragmented MP4 fixtures and a large file
  performance test. Libraries like `transmux` provide demux/mux options,
  but may have different memory behavior or maturity.
- The lab intentionally keeps the tested H.264/AAC codec selection used by
  Akula's native Windows Media Foundation player. No video transcoding is
  attempted. Unsupported codecs must fail with a clear message.
- QuickJS in yt-dlp uses temporary EJS scripts and has a documented TOCTOU
  risk; on Windows it must run in a private directory with guarded ACL and
  no untrusted temp path.
- **No current proof of actual YouTube challenge success.** A pinned JS
  runtime parsing check is not a substitute for a legal, controlled extractor
  integration test.
- Current `yt-dlp.exe` remains a Win10+ upstream executable. The native
  Akula Windows 7 playback fallback in #963 does not make yt-dlp Win7-safe.
- LGPL/GPL obligations of the abandoned *heavy* lab are not automatically
  inherited by this prototype. Still review `mp4` (MIT), QuickJS-NG
  (MIT), yt-dlp executable redistribution/bundled third-party licenses, and
  supported operating systems.
- No release, installer, cloud upload, or actual customer media may be
  produced by this experiment.

Links: https://docs.rs/mp4/0.14.0/mp4/;
https://github.com/quickjs-ng/quickjs/releases/tag/v0.18.0;
https://github.com/yt-dlp/yt-dlp/wiki/EJS.
