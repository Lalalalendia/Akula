# VIDEO-17 — genuine unsigned NSIS with the complete local video toolchain

## Goal

VIDEO-16 proved a physically clicked Windows video download, app restart,
real Play click, a decoded frame, and independent FFmpeg decoding. It used
a manually staged Akula EXE, not the product's installer. VIDEO-17 must
prove that **a genuine NSIS install** delivers the same exact toolchain and
that it can be uninstalled without deleting local video or synchronized notes.

This is a **non-release lab**, not a shipping or publisher-signed installer.

## Design and isolation

- Copy the ordinary `aquilum-app/native/packaging/akula-windows.json` into
  an isolated `akula-windows-video17-lab.json` that declares exactly five
  binaries: `akula-app`, `shiva-pdf-preview`, `akula-video-mux`,
  `akula-yt-dlp`, `akula-quickjs`. Product ID and version stay identical.
  The original two-binary shipping config stays unchanged.
- Build the source-reviewed Rust mux from `labs/video-lite/Cargo.lock`
  and embed its actual SHA-256 into an opt-in Windows GUI test build using
  `--features installed,video-rust-bundle,video-ui-e2e`.
- Verify yt-dlp 2026.10.26 and QuickJS-NG 0.18.0 by exact SHA-256 and
  byte counts. Build Shiva from exact commit
  `40b72a90b1dac39812c77c7321e57d613a81cb51`, matching the existing
  Windows beta acceptance. Build genuine NSIS using pinned
  `cargo-packager 0.11.8` and `--release`, with no signing or publishing.
- Install the NSIS silently on a **clean GitHub-hosted Windows runner**.
  Require Akula's HKCU uninstall identity/version and verify byte-identical
  SHA-256 for all five installed EXEs, not merely the installer's exit code.
- Create a synthetic Aquilum install/data/registry sentinel and require it
  to remain untouched during both installation and uninstallation.
- Reuse VIDEO-16's actual Windows `SendInput`/UIA GUI scenario against the
  installed `%LOCALAPPDATA%\Akula\akula-app.exe`. Its test-only network
  redirection serves an exact 6-second H.264/AAC DASH fixture from
  `127.0.0.1`; no YouTube traffic occurs. Require pre-click no download,
  real Download click, local MP4, unchanged synced Markdown and vault, close,
  reopen, real Play click, and decoded frame.
- Run NSIS `uninstall.exe /S` on the same runner. Verify all five installed
  EXEs and Akula's uninstall registry entry are removed, Aquilum sentinels
  remain untouched, and the local video and note are preserved.
- Publish **only** the short synthetic MP4 as a 2-day CI artifact, for a
  separate Ubuntu FFmpeg H.264/AAC decode check. Never publish NSIS or the
  third-party binaries.

## Test gate

`.github/workflows/akula-video17-nsis-installed-video.yml` is the
dedicated VIDEO-17 workflow; it subscribes only to its own new scripts,
config and workflow, minimizing existing source CI fanout.
It can also run manually. Require exact-head boundary, fixture, genuine
Windows installer/GUI/uninstaller and independent decoder checks before
merging into VIDEO-03. After merge, require parent Application checks
including the no-growth ratchet, full Windows beta and video regression.

## Explicit release blockers

- The VIDEO-17 installer is **unsigned** and embeds a test-only transport
  feature; it is not an authorized artifact for users. The real publisher
  signing and protected Windows NSIS workflow stay separate and unchanged.
- An install under ordinary per-user `%LOCALAPPDATA%` does not prove
  tamper-resistant ACLs or eliminate the SHA-256-to-CreateProcess replacement
  race. VIDEO-18 must use the existing Akula signing/update architecture,
  avoid a second updater, prove protected sidecars and crash/recovery
  behavior, and test a genuinely signed installer.
- Windows 7 cannot run the current pinned yt-dlp binary. VIDEO-19 needs a
  separate support decision and compatibility evidence.
- No authenticated live YouTube extraction, full media-format coverage,
  DRM bypass, license redistribution approval or public release is claimed.
- No user files, personal vaults, production GitHub Releases, or `main`
  must be changed by this lab.
