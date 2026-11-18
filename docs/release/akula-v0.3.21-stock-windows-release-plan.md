# Akula v0.3.21 — stock Windows release scope and owner checklist

**Status: release preparation; NOT PUBLISHED.** The latest public Windows release at the time of drafting (2026-11-19) is `v0.3.20`. Do not call a merge, a PR CI pass, or a VIDEO-18 laboratory build a published release.

## Intended user-facing changes since v0.3.20

- **Study collections:** bind authorized Markdown-table columns to named study collections, manage membership, rename/delete collections without editing the source note, and combine selected authorized tables from multiple notes.
- **Exercises in the native UI:** reveal-answer flashcards, multiple choice, match-the-pair exercises and typed-answer checks. All question sources are revalidated on access. **Not yet available:** spaced repetition/SRS, durable answer history, progress analytics, full game UI or cloud synchronization of study content.
- **Text highlighting:** six colors (yellow, green, violet, blue, orange and pink) in the note editor, with local revisioned annotation metadata stored separately from Markdown. Stale/ambiguous anchors fail closed. **Not yet available:** cross-device annotation sync.
- **YouTube preview:** recognized single-video URLs show a native thumbnail and media controls. On the stock installer video download needs an **independently installed** `yt-dlp` executable; there is no bundled video runtime. Do **not** promise out-of-box YouTube downloading or the protected five-executable updater.

## Explicit packaging boundary

The normal publisher-controlled `aquilum-app/native/packaging/akula-windows.json` contains **only**:

1. `akula-app` — main binary;
2. `shiva-pdf-preview` — PDF sidecar.

The VIDEO-17/18 five-EXE NSIS packages use separate lab manifests and ephemeral keys. Their real signature checks, Windows Program Files ACL denials and four process-kill rollback scenarios are valuable non-release evidence, **not** proof of shipping auto-rollback or release-signing custody.

Normal signed Windows `akula-win7-signed-stage.ps1` uses the `installed,s3-pair-beta` Cargo features and an audited two-file package. It must not enable `video-rust-bundle` or `video-ui-e2e`. The current published 0.3.20 NSIS notes explicitly state that **automatic NSIS crash rollback is unavailable**; keep the same clear limitation on the 0.3.21 release notes.

**VIDEO-18 RELEASE HOLD remains ON** for the protected five-EXE video bundle; `VIDEO_FIVE_CHANNEL_READY=false`. No owner signing key, production updater, release permissions, installers or end-user installations are modified by this planning document.

## Required final release execution (owner/protected environment only)

1. Select an approved **exact commit on `main`** after all desired 0.3.21 learning and annotation PRs have merged. Record its SHA. Do not release from a moving/ref-only review.
2. Ensure the native, core, Cargo.lock and `akula-windows.json` versions are all **0.3.21**; run `node scripts/akula-version.cjs check` and `node scripts/akula-version.cjs assert-newer v0.3.20`. Verify v0.3.21 does not already exist.
3. Confirm `Main PR gate`, applicable Windows GUI tests, release-source custody and VIDEO-18 bundle-HOLD checks are successful at the chosen commit. A green experimental VIDEO-18 build **does not replace** a signed release candidate.
4. In GitHub Actions, open `Akula Windows signed candidate (review only)`, use **Run workflow on main**, input `release_version=0.3.21`, and initially set **`publish_beta=false`**. This is a manual protected operation, not an automated PR side effect.
5. Require real signed two-file NSIS construction, actual installed Windows tests, **published v0.3.20 → signed v0.3.21 upgrade test**, source/lock provenance, owner-key signature and independent signature verification. If a job fails, fix the cause and repeat candidate verification. Do not publish failed or merely fabricated receipt output.
6. After reviewing the exact signed bytes and verification results, the authorized owner may make a **separate conscious decision** to run the protected main-only workflow with `publish_beta=true`. Only then verify the GitHub Release `v0.3.21`, signature files, `latest.json`, current release tag and downloadable asset digests.
7. Test on a sacrificial Windows install with separately backed-up user notes/settings; the stock updater has **no guaranteed automatic NSIS crash rollback**. Keep previous signed v0.3.20 available for explicit reinstall.

## Next video work — separate release track

Do not expand the updater merely to ship v0.3.21. First make the video UX honest on an ordinary stock installation and accept genuine source/legal, network-failure and offline-playback cases. Decide between a separately installed opt-in protected video package or waiting for a fully approved five-EXE bundle **before promising download without manual setup**. Genuine Windows reboot/power-loss and publisher-key activation remain VIDEO-18 release admission work, not blockers for the stock two-EXE release.

## Short public release notes (English)

### Akula 0.3.21 — study workflows and editor annotations

- Create named study collections from Markdown tables and practice across selected authorized notes.
- Study with flashcards, multiple-choice questions, matching exercises and typed answers.
- Highlight selected note text using six colors while keeping source Markdown unchanged.
- Preview single YouTube videos in notes; offline downloading still requires external tooling and is not bundled.
- Numerous security and compatibility checks for the non-shipping video update laboratory.

**Windows signed two-file NSIS update. No automatic NSIS crash rollback; the protected five-component video updater is not included.**