# VIDEO-16 — real Windows GUI acceptance against localhost DASH

## Acceptance target

VIDEO-12 proves Akula's native downloader (yt-dlp + QuickJS + Rust mux) can
publish a decodable MP4, but directly calls Rust functions. VIDEO-15 proves
the experimental Windows sidecars are exactly pinned, but does not prove
that the actual editor button triggers that pipeline.

VIDEO-16 adds **actual OS pointer input** in a clean GitHub-hosted Windows
desktop. It uses a staged, standalone candidate in
`%LOCALAPPDATA%\Akula\akula-app.exe`; it does **not** build or run an NSIS
installer and must not be described as installer acceptance.

## Real GUI path

1. Compile an opt-in `installed,video-rust-bundle,video-ui-e2e` Windows
   candidate with a build-embedded Rust mux SHA-256. The last feature is
   absent from default `installed` and all publication configs.
2. Stage Akula, source-built Rust mux, official SHA-verified yt-dlp 2026.10.26
   and QuickJS-NG v0.18.0 together, in a disposable hosted runner only.
3. Produce a six-second **synthetic** two-track DASH fixture on Ubuntu.
   Python serves its relative segments from `127.0.0.1`. The GUI note
   contains exactly one standalone validated YouTube URL but the *test-only*
   transport seam replaces that URL with the loopback MPD. Neither YouTube
   nor its thumbnail CDN is requested. Ordinary builds do not have this seam.
4. Start the real Akula executable against a new temporary vault/data
   directory. Open the note in its actual editor and confirm no MP4 exists.
5. Enumerate the Windows UIA editor bounds, move the physical OS cursor
   through that area, and wait for the compiled-only hit-test witness
   `HOVER_DOWNLOAD`. Issue a real foreground-PID-checked SendInput mouse
   down/up at that exact location. Verify `CLICK_DOWNLOAD` from
   `TextEditor::media_press` and a newly published MP4 under the **device
   data directory only**, not in the note or vault.
6. Close Akula gracefully; clear the transient witness log; reopen the
   **same note and profile**. Find `HOVER_LOCAL_PLAY` via OS pointer
   movement, click, and require `CLICK_LOCAL_PLAY` and at least one
   `DECODED_FRAME` from the live post-paint rendering path.
7. Check original Markdown SHA-256 throughout. Only the new synthetic MP4
   is exported as short-retention CI evidence. An independent Ubuntu job
   decodes H.264 and AAC and counts real frames.

## Non-shipping controls

- The `video-ui-e2e` feature implies `video-rust-bundle` and is not
  enabled in any public flavor. Its transport override requires both
  `GITHUB_ACTIONS=true`, `RUNNER_OS=Windows`,
  `RUNNER_ENVIRONMENT=github-hosted`, an exact hardcoded fixture video ID,
  and literal `127.0.0.1:<port>/manifest.mpd` (no arbitrary hostname,
  port 0, path or query).
- A blank synthetic thumbnail is deliberately used in this feature to
  prevent even metadata/thumbnail network requests to a third-party CDN.
- GUI event witnesses are only emitted inside the same opt-in feature and
  allowlisted for `RUNNER_TEMP/video16-events.log`. They cannot be used
  to invoke actions; the Windows script cannot call Rust methods.
- The normal `installed` feature and packaging manifest remain unchanged,
  no S3/Dropbox writes are used, no installer, release or user paths touched.

## What remains after a green VIDEO-16

A disposable staged binary is not a signed installer. Protected installation
ACLs, a signed/updateable chain for all four executables, recovery/rollback,
licensing approval, actual site content compatibility, and Windows 7 strategy
remain separate release gates. A SOURCE file SHA alone does not prove
bit-for-bit reproducible Windows builds. Do not merge VIDEO-03 into main or
ship a public release based solely on this candidate.
