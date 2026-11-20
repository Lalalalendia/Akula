# VIDEO-22: real YouTube E2E for Akula v0.3.21 (NONRELEASE)

## Why this is distinct from VIDEO-16/20/21
- VIDEO-16 exercised physical Win32 GUI clicks, but replaced the YouTube URL
  with a synthetic local DASH fixture in a dedicated test-only feature.
- VIDEO-20 and VIDEO-21 verified user consent, official download, SHA-256 and
  removal of optional yt-dlp / QuickJS on a real Windows runner, but stopped
  before downloading any YouTube video.
- VIDEO-22 exercises the **ordinary installed download adapter** against the
  **actual YouTube service**, with no localhost transport override. It accepts
  a local MP4 only when the physical GUI Download click and subsequent
  restarted local playback are both witnessed.

## Windows test only
- The original youtube-dl sample `BaW_jenozKc` was deleted and must not be used.
- Source is selected strictly from two uploader-authored **CC BY 3.0** AnnexTube
  Testing clips (2 seconds `BZeKDYqsuj0` and 1 second `GhGQV_enM8M`).
  The Windows preflight verifies the exact progressive format supported by
  Akula, records the selected ID, then uses that same URL in the real GUI.
  If neither offers a usable stream, CI fails; no synthetic fallback is allowed.
- Runner: GitHub-hosted Windows 2025, independent disposable vault/data profile.
- Build: `installed,video-tools-gui-e2e`. The latter is an empty feature
  allowing observation-only pointer/decoder witnesses; it is NOT used in normal
  NSIS packaging and does not enable `video-rust-bundle` or `video-ui-e2e`.
- Exact official yt-dlp 2026.10.26 and QuickJS-NG 0.18.0 bytes are staged
  in the same local tool paths previously verified by the VIDEO-20/21 real
  Settings installation tests. This E2E focuses on *use after installation*,
  and does not claim to repeat the Settings installation in the same process.
- The GUI note contains an actual YouTube URL. No localhost MPD, mock HTTP
  transport, or fake `yt-dlp` executable is allowed.
- Real foreground mouse events click the editor's media Download control,
  then verify a nonempty device-local MP4 with that YouTube ID.
- Close and reopen Akula using the exact same profile, click Play through
  the real editor UI, and require a decoded native frame. SHA-256 of Markdown
  must not change. There must be no media files in the synchronized vault.
- The test leaves no uploaded MP4 artifact or public release. A successful
  log may include its SHA-256/length; files are removed from the runner.

## Failure handling
An extraction failure from YouTube, a missing progressive AVC/AAC MP4,
network/IP blocking on GitHub-hosted runners, unsupported Javascript
challenge, or native decoder failure are **NOT** accepted as PASS.
Distinguish these causes in job logs. When YouTube blocks hosted traffic,
repeat the same genuine GUI scenario in an authorized isolated Windows
acceptance environment with external network access. Do not substitute a
synthetic localhost fixture and call it a live-YouTube acceptance.

## Release gating
Successful VIDEO-22 is a video product gate, not a signed release.
After it passes, run the owner-protected two-executable signed candidate
workflow for the exact approved main commit, first with
`release_version=0.3.21` and `publish_beta=false`, and perform the
v0.3.20→v0.3.21 installed upgrade and rollback/manual recovery tests.
Public release always requires a separate authorization.
