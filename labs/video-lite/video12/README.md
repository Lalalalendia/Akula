# VIDEO-12: native Akula pipeline against real tools (non-release)

The preceding VIDEO-10 experiment used **real yt-dlp, QuickJS and the Rust
MP4 muxer**, but its Python script called them directly. It did not run the
download orchestration actually used by Akula's native application.

VIDEO-12 closes that gap without touching YouTube or a user workspace.

## Exact test

1. Ubuntu CI generates a synthetic six-second H.264 + AAC DASH manifest and
   separate segments from the existing repository-owned test asset.
2. Windows CI builds the corrected `dash mux-pair` CLI from source and
   downloads **official pinned** yt-dlp 2026.10.26 and QuickJS-NG 0.18.0 into
   disposable runner-only directories, verifying both exact SHA-256 values.
3. A private Python HTTP server binds to `127.0.0.1` and serves only these
   synthetic files. A Windows-only Rust test calls Akula's actual
   `download_rust_from_source` path with a known single-video note URL but
   with a **test-only local transport URL**. Production
   `download_rust` always supplies the same validated YouTube URL for
   both arguments, so no production caller can substitute its transport.
4. The native downloader launches genuine yt-dlp twice for the same exact
   AVC and AAC selectors and progress callback, then calls the Rust v2
   muxer, verifies its nonzero-duration protocol receipt and atomically
   publishes the result in a private local media library.
5. Assertions prove deduplication, clean staging directory, unchanged
   Markdown note, and no files inside the registered synchronized vault.
6. The resulting synthetic MP4 is uploaded for reference-only FFprobe and
   FFmpeg decoding on Linux, requiring a positive duration and at least
   30 actual decoded frames **for each** audio and video track.

The workflow runs the native test with `--features installed` but does
**not** publish an installer or distribute any third-party executables.

## Boundaries

This is a native-process integration test, **not a click-through GUI test**.
It is neither proof that YouTube's current JS challenges are supported, nor
permission to download content without authorization. No real YouTube
request occurs. Rust muxer packaging, trusted installation ACL, and
Windows 7 compatibility remain unresolved. VIDEO-03 remains Draft until
separate installed-GUI and release-policy acceptance.

The source URL substitution is an explicitly private test seam; it is not
reachable through the Akula UI, note links or sync engine. Removing that
seam would prevent realistic offline acceptance and risk false confidence
in a downloader checked only with mocks.
