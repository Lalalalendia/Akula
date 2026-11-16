# VIDEO-18 — release supply-chain admission for local video (HOLD)

Status: **DRAFT / SECURITY HOLD / NO RELEASE**.

## Evidence already available

- VIDEO-13–15 pinned and checked the source-built Rust mux, official
  yt-dlp.exe and QuickJS-NG qjs.exe before launching them in the experimental
  `video-rust-bundle` Windows feature.
- VIDEO-16 accepted actual Windows pointer Download, local MP4, process
  restart, Play and a decoded frame against synthetic localhost DASH.
- VIDEO-17 #978 accepted an actual **unsigned** five-binary Akula NSIS
  install/uninstall, independent hash checks for all installed executables,
  preservation of private video/Markdown and independent FFmpeg decode
  (Actions 43908369394, exact head `8f7f3a31`).
- These tests say nothing about the publisher signing key or the safety of
  a malicious same-user process on the local device. No public release
  and no claim of Windows 7 yt-dlp compatibility is authorized.

## The existing trusted path we MUST reuse

`.github/workflows/akula-windows-signed-candidate.yml` already has:
1. `main`-only manual release dispatch and protected `akula-release`
   environment, default `publish_beta=false`.
2. Real NSIS installation/test before signing.
3. Owner-key `cargo packager signer sign` for the **exact** NSIS and
   `latest.json`, an independent verification job, and separately approved
   publication; source provenance, version, Cargo.lock and package digest.
4. Native `updater::release` authenticated `SignedCandidate` admission,
   not raw user-supplied EXE input.

Its **shipping** `akula-windows.json` presently lists only
`akula-app` and `shiva-pdf-preview`. The five-binary VIDEO-17 JSON is a
separate, unsigned laboratory. Introducing video into signed Windows NSIS
without extending that existing owner-controlled path is forbidden.

The existing Windows `updater/apply.rs` launches a verified signed NSIS
but has no generation-bound first-render confirmation or automatic durable
rollback after failures. Its current `channel_ready()` is true for
`windows + installed`: the older design notes describing an unconditional
HOLD do not, by themselves, disable that code. **Do not infer that the
current update transaction is crash-safe from successful beta CI.**

## Non-release guard added in VIDEO-18a

`.github/scripts/video18-release-bundle-hold.test.cjs` deliberately
enforces that the public installer still lists **two** binaries; that the
isolated VIDEO-17 lab lists **five** with no other identity/version drift;
that `installed` does not include `video-rust-bundle` or `video-ui-e2e`;
and that the signed-candidate workflow retains its main-only/manual protected
gate, owner signatures and independent verification. Three mutation tests
ensure those assertions actually reject unsafe promotion.

This is a static **regression gate**, not a signature, ACL proof or installer
attestation. It intentionally has to be revised by an owner-reviewed change
when the full five-component release architecture and negative tests exist.

## Mandatory VIDEO-18b acceptance before enabling shipping video

1. **Distribution decision:** either (a) a Windows 10/11-only protected and
   signed installation root with an explicit signed migration/rollback path,
   or (b) a signed OS-managed package (e.g. MSIX) supporting the required
   Windows versions. The current mutable per-user LocalAppData layout
   cannot promise protection against another process with the *same user's*
   access token. Hash checks and signed downloads do not change that fact.
2. **Signature and dependencies:** bind the exact five helper binaries to
   an independently reviewed Akula signed package and its manifest, with
   source/version/license/SBOM evidence. Determine whether Authenticode
   signing of all PE files is required, as separate from the existing
   Minisign signature on the installer and metadata. Never download runtime
   components silently or fall back to PATH or unpinned EXEs.
3. **Activation and update safety:** authenticated signed candidate,
   protected source/destination, single OS lock, retained previous
   generation, journal, bounded generation-matched first-GUI ACK, crash
   recovery, rollbacks of all five binaries as **one unit**, downgrade
   protection, and explicit handling of open files and AV interference.
   Reuse the existing signed release/update system, not a second updater.
4. **Attacker tests:** reject replaced/tampered yt-dlp/QuickJS/mux, symbolic
   links, reparse points, parent-directory substitution, and updates stopped
   between every journal phase. Show what an equal-privilege local attacker
   can still do, rather than asserting impossible tamper immunity.
5. **OS compatibility:** current pinned yt-dlp is a Windows 10+ build.
   Keep Akula's Windows 7 baseline independent, with video feature held
   or disabled explicitly there unless a separately accepted compatible
   runtime exists; never quietly ship the VIDEO-17 test build as Win7.
6. **Product acceptance:** genuinely signed/protected installed GUI, download,
   cancellation, restart, offline playback, network errors, no sync,
   upgrade/rollback and uninstall on supported systems. Distinguish
   synthetic DASH proof from real authorized site compatibility, and never
   bypass DRM or private content access.

Owner approval and a separately protected release are required after all
these gates succeed. This file does not enable `video-rust-bundle` for
ordinary users and does not authorize publishing any installer.
