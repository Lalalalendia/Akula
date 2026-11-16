# Akula VIDEO-08 — staged FFmpeg-free downloader integration

**Draft integration into VIDEO-03 only. No public release or installer change.**

This change keeps the existing **explicit user click** queue and adds an
**opt-in** Rust mux path selected when `AKULA_RUST_MUX_BIN` points to an
absolute local executable. Without that opt-in, the original VIDEO-03
download behavior is unchanged.

## Flow

1. After an explicit **Download** click, the queue validates a single
   YouTube URL and opens a local media library outside all registered vaults.
2. A private, same-volume staging directory is created inside that library.
3. yt-dlp downloads a **video-only** AVC MP4 at up to 720p, then an
   **audio-only** AAC M4A into that directory, using explicit formats and
   `--fixup never`. Neither invocation requests an FFmpeg merge.
4. The configured Rust muxer is called with `mux-pair` and the two tracks.
   It can accept progressive and fragmented MP4 track containers, subject
   to the restricted supported codecs and timebases.
5. Akula validates the result, checks its size quota, flushes the writable
   file handle, then publishes the new `<video-id>.mp4` with a no-clobber
   hard link. The staging directory is removed, on success and failure.
6. A second note uses the same local media file. On another device, only
   the original Markdown URL and preview sync — no media bytes.

The route is intentionally opt-in because a real pinned QuickJS/yt-dlp/Rust
sidecar distribution, Windows 7 policy, EJS security and genuine installed
GUI acceptance still require work. No code automatically downloads a tool.

## Security and failure cases

- Only a validated YouTube single-video URL reaches yt-dlp; no shell.
- Download paths are fixed, and the staging directory is outside vaults.
- yt-dlp config, cookies, playlists, arbitrary execution and post-fixups
  are disabled. Size caps apply to each stream and their combined bytes.
- The muxer path must be absolute, a regular file and not a reparse point.
- Only regular, non-empty `video.mp4`, `audio.m4a`, and merged MP4 are
  accepted; symlinks/reparse-points and oversized outputs are rejected.
- Failed or cancelled operations do not change Markdown or publish media.
- A pre-existing file is reused; an atomic hard link never overwrites it.

## Remaining release gates

- Confirm yt-dlp 2026.10.26 really exposes separate H264 and AAC tracks
  and that QuickJS-NG can solve the current permitted-source challenges.
- Supply a pinned, checksum-verified yt-dlp + QuickJS-NG + Rust muxer
  sidecar from a trusted installation directory, with license review and
  safe EJS temporary execution rules. No PATH hijacking or unpinned updates.
- Run a Windows 10/11 installed GUI click → progress → video/audio download
  → mux → restart → native playback acceptance with logs and hashes.
- Windows 7 SP1 needs an explicit strategy: current upstream yt-dlp.exe is
  Win10+. Native video playback on Win7 does not provide downloading.
- Check high-duration content, malformed fMP4, missing streams, storage
  exhaustion, power loss and same-device concurrent access.

This change is **not** evidence of any of the above. It proves the offline
control flow using deterministic fake downloaders and fake muxers.
