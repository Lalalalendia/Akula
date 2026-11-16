# VIDEO-15 — pinned side-by-side yt-dlp and QuickJS (non-release)

## Problem

The opt-in VIDEO-13 Rust mux candidate used the binary shipped beside Akula,
but still required the user to supply `AKULA_QUICKJS_BIN` and a
`AKULA_YT_DLP_BIN` path (or PATH lookup) from the queue. A one-click
installed application cannot rely on arbitrary executable locations.
An MP4 receipt cannot authenticate an arbitrary executable.

## Implementation

In the **non-default** `video-rust-bundle` feature on Windows only:

- Resolve `akula-video-mux.exe`, `akula-yt-dlp.exe` and
  `akula-quickjs.exe` as siblings of `std::env::current_exe()`.
- Require the build-time SHA-256 of the Rust mux (VIDEO-13), the exact
  pinned upstream SHA-256/byte count of yt-dlp 2026.10.26 and QuickJS-NG
  v0.18.0. Existing `safe_file` also rejects missing, empty, oversized,
  aliased, symlink or Windows reparse-point files before spawning.
- Ignore the queue's PATH/environment-selected yt-dlp path in this feature.
  `AKULA_QUICKJS_BIN` is no longer accepted in the Rust path. The
  legacy, normally installed code path is unchanged; it does not
  automatically download or load new media.
- Keep `--ignore-config --no-plugin-dirs --no-remote-components
  --no-js-runtimes --js-runtimes quickjs:<verified path>` for yt-dlp,
  and retain device-local staging, atomically published MP4 and v2 mux
  receipt checks.

VIDEO-13's existing non-release Windows workflow now downloads both exact
official executable bytes in a disposable runner, checks their SHA-256 and
size **before** use, runs positive and altered-file Rust tests, and
stages all four binaries side by side. It does not create an installer or
upload artifacts.

## Explicitly NOT proved

- Neither the normal Akula installer nor default `installed` feature
  distributes these tools. The experimental candidate is **not a release**.
- The upstream yt-dlp.exe remains Windows 10+ and does not establish a
  Windows 7 downloader. Native playback fallback and extractor support
  are different problems.
- A hash check immediately before a process spawn is insufficient against
  same-user directory replacement races. The final installation needs
  protected ACLs, a signed package/update chain and tamper testing at
  each boundary.
- Pure-Rust MP4 mux only supports approved H.264/AAC MP4 tracks; arbitrary
  WebM/AV1 or DRM media is not supported. No third-party YouTube network
  request or playback compatibility claim is introduced.
- Full installed Windows GUI click → progress → close/restart → playback
  still requires a separate acceptance test.

Do not merge this draft until exact-head Windows VIDEO-13, VIDEO-12 and
Application checks pass. Do not modify `main` or publish a release.
