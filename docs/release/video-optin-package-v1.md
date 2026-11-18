# VIDEO-PACKAGE V1 — optional Windows video tooling without a second auto-updater

**Decision (2026-11-19):** prioritize a **separate opt-in, publisher-signed Windows 10/11 video-tools package**, distributed and updated **manually**, rather than extending the VIDEO-18 protected five-EXE automatic updater now. This is an implementation target, **not** release or signing approval.

## Baseline

- Stock Akula **0.3.21** remains the Win7-compatible, owner-signed **two-EXE** package (`akula-app.exe`, `shiva-pdf-preview.exe`). Its existing update flow is not modified and does **not** acquire automatic crash rollback.
- Stock Akula can preview YouTube links. Today offline download requires a separately installed `yt-dlp`; do not advertise the feature as working out of the box.
- VIDEO-17 and VIDEO-18 proved protected five-EXE installs, signed temporary manifests, real version `0.3.20 → 0.3.21`, bounded GUI ACK and four external process kills with full five-file recovery. Those are non-release GitHub runner laboratories, **not** publisher-key distribution.
- **VIDEO-18 RELEASE HOLD / `VIDEO_FIVE_CHANNEL_READY=false` stays ON** for the integrated five-EXE bundle.

## V1 installation choice

Create a separate optional protected **Akula Video Tools** package with **its own installer/uninstaller identity**, version, distribution metadata and protected Program Files directory. It contains exactly the three media runtimes required by the opt-in pipeline:

| Component | Role |
| --- | --- |
| `akula-video-mux.exe` | source-pinned Rust H264/AAC MP4 remux |
| `akula-yt-dlp.exe` | pinned, separately license-reviewed yt-dlp |
| `akula-quickjs.exe` | pinned JavaScript runtime required by yt-dlp |

The ordinary Akula NSIS must **never** install, replace or uninstall those three files. Conversely, the video package must not replace the standard Akula executable, the PDF helper or user data. Windows 7/8 keep the stock Akula without these media tools. No runtime download of executables.

**Reason for this choice:** it separates video runtime release cadence from the user's note editor and avoids claiming that the stock two-EXE updater can transact a five-file package. It does **not** eliminate signature validation, OS protection or user-visible install/reinstall compatibility testing.

## Work plan and acceptance

1. **Package identity:** write a distinct NSIS manifest for the three executables and explicit protected install/uninstall behavior. Reject overlapping standard package roots, identity confusion, junctions and normal-user binary replacement. No second auto-updater.
2. **Trust and discovery:** the native app must locate the separately protected package through an *owner-approved signed manifest and fixed product identity*. Verify exact expected version, source hashes and all three PE digests. Reject arbitrary `PATH` or `AKULA_RUST_MUX_BIN` fallback in packaged mode; do not treat checking the filesystem path alone as sufficient authority.
3. **User flow:** in the ordinary Akula UI, show whether Video Tools is present and authenticated, provide a clear optional install/update instruction, then enable Download only with the validated set. An absent package must result in an informative, localized notice, not a silent queue failure.
4. **Functional tests:** real signed installer under an unprivileged user after installation, real authorized YouTube download, exact AVC/AAC decode, offline playback after restart, quota/cancel/network errors, no Vault contamination, stable notes/settings, clean package uninstall/reinstall and old package rejection.
5. **Owner signing and legal review:** source/version/license/SBOM review for each runtime; sign the exact package and metadata under publisher-controlled credentials, independently verify; test Windows 10 and 11. Never equate ephemeral CI Minisign with publisher signing.
6. **Manual updates:** on a later Video Tools release, prompt the user to close Akula before replacing the three tools. Treat a failed manual update as a failure requiring explicit reinstall; do not promise automatic durable rollback until separately accepted.

## Non-goals and release blockers

- No automatic five-EXE update or Windows reboot/power-cut tests as part of stock 0.3.21 release.
- No arbitrary online installer fetching, automatic video-runtime self-update, DRM bypass, private content access or opaque third-party binary approval.
- No shipping Video Tools until end-to-end signed protected installation, host verification and **real user-mode tests** succeed. Any later integrated automatic updater must reopen VIDEO-18's remaining reboot/power-loss, locked-file and publisher-chain gates.

**Priority:** publish a verified standard 0.3.21 first; then implement opt-in Video Tools without coupling its acceptance to the stock updater.