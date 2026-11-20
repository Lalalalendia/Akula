# VIDEO-20 — Akula installs alone; yt-dlp is optional in Settings

## Release-focused product contract

**User decision (20 November 2026):** stop bundling yt-dlp in the Akula installer; offer an explicit **Settings → System → Video: yt-dlp → Download yt-dlp** action. The prior three-executable signed Video Tools bundle is not the product shipping mechanism. Its `VIDEO-18 RELEASE HOLD` stays intact and no publisher key is silently enabled.

The existing stock Windows packaging config (`aquilum-app/native/packaging/akula-windows.json`) already includes only `akula-app` and `shiva-pdf-preview`; do not change this. The `video17-lab` five-EXE installer remains a non-shipping QA fixture only.

## Windows interaction

1. Fresh Akula: status **Not installed**; clicking a YouTube Download card shows Settings guidance, without starting a process or network download.
2. Settings → System: a manual button and native Yes/No confirmation precede any fetch. No start-up auto install, no periodic background updates, no use of a system `yt-dlp` from `PATH`, env variables, synchronized vaults, or roaming profile.
3. User consent fetches exact `yt-dlp.exe@2026.10.26` directly from the official `yt-dlp/yt-dlp` GitHub release over HTTPS. Expected length is **23,575,744 bytes** and expected SHA-256 is `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d`. A changed release is rejected. Any failed network transfer, wrong size or digest leaves the public target absent.
4. Download runs on a background thread, reports bounded progress, stages in the fixed app-local folder under `%LOCALAPPDATA%`, and publishes via same-volume no-clobber hard-link after checksum and fsync. The program never writes tool bytes to notes, the video library, an updater staging area or an Akula installer.
5. Only a re-verified, exact-hash local tool can be passed to the installed download queue, which uses no shell, no plugin directories, no user yt-dlp configuration, no remote components and single-video restrictions. When unavailable, the queue refuses without touching the local library.
6. Settings exposes **Remove yt-dlp** after an explicit confirmation. The operation does not delete saved videos or notes. Any invalid file must be removed and reinstalled manually; it will not be executed or overwritten by an unrelated installer.
7. This first product path chooses a **single combined AVC/AAC MP4 track up to 720p** instead of separate DASH tracks, avoiding an FFmpeg requirement. This can mean lower resolution or failure when no compatible progressive format exists; video completeness and JS-dependent YouTube extraction still need independent Windows end-to-end acceptance.

## Security and release boundaries

- The separately downloaded program is a third-party executable installed only after the user agrees; a SHA-256 hard pin is *byte integrity*, not a signed software supply-chain or full independent legal approval.
- The outer Akula installer must still follow its own Windows signing, updater, real GUI and rollback release gates. No public release is made by merging these changes.
- `VIDEO-18 RELEASE HOLD` for the former **bundled** three-component toolchain remains; its protected-root, publisher-key and license research records are unchanged.
- A downloaded third-party executable may include code covered by its own license; offering its download does not eliminate applicable license/notice duties. Links to its upstream project and licenses should remain accessible.
- Do not claim 1080p DASH/mux, a first-frame YouTube test, Windows 7 support, or authenticated YouTube download acceptance without specific evidence.

## Acceptance before shipping

- Rust Core and installed Windows compilation/tests; UI state + confirmation + install/invalid/remove tests; untrusted `PATH` ignored.
- Pinned HTTPS fetch tested against an actual official GitHub release and SHA-256, without auto-install or executable upload.
- Real Windows click through Settings, reconnect, progress, then video download; local library survives app restart and removal of yt-dlp.
- Stock two-executable NSIS check, separate Akula publisher-signing and updater integrity checks.
