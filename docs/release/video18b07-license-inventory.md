# VIDEO-18b-07 — Rust transitive license inventory (NON-RELEASE)

**Status: metadata evidence only. VIDEO-18 security HOLD remains in force.**

This experiment builds no Rust binaries and never touches Akula's shipping NSIS,
updater, signed distribution or user machines. A pull-request-only Linux
workflow uses `cargo metadata --locked --all-features` on three exact source
workspaces: Akula's `aquilum-native`, the first-party Rust fMP4 mux
`akula-video-lite-lab`, and Shiva CLI at pinned source commit
`cb542d30d2c18a18b1ee457f5158d5b0f551757e`.

The script walks each package's full Cargo dependency graph, retains the
dependency name, version, immutable source identifier, SPDX license
**declaration** (when present) and license-file name, and marks entries
with neither license declaration nor license file as
`BLOCKED_MISSING_LICENSE_METADATA`. Repeated packages are deduplicated
across workspaces while preserving which products use them.

The two upstream executables (yt-dlp and QuickJS) are included through the
existing non-release five-EXE allowlist. The test rejects missing dependency
graph nodes, malformed source SHA, a removed VIDEO-18 release HOLD, lost
workflow source subscriptions and attempts to add manual publishing.

The artifact is a JSON **review queue**, retained for one day. No executables,
source archives, secret credentials or signed artifacts are uploaded.

## Limits of this evidence

- A Cargo `license` string is **a claimed SPDX expression**, not verified
  copyright ownership, a copy of its full license, or proof that redistributing
  a compiled bundle meets notice obligations.
- All-features dependency closure is conservative: it may include crates
  unavailable in the actual Windows 10/11 release, and can still omit bundled
  resources, dynamically loaded libraries, non-Cargo subcomponents and
  dependencies embedded inside released upstream binaries.
- Missing metadata is an actionable blocker. Non-missing metadata is still
  marked `REVIEW_SPDX_AND_LICENSE_NOTICE`; no automated result implies
  licensing approval or relieves independent legal/source review.
- This report does not license a real five-binary NSIS, sign its contents or
  solve race-resistant installation and transactional updates.

## Next required actions

1. Triage each missing or ambiguous SPDX declaration and gather original
   license texts and notices from pinned upstream sources.
2. Build a distributable NOTICE/SBOM bundle, audit dynamic/bundled binary
   components, and separately decide which licensing obligations apply.
3. Continue protected signed five-file NSIS identity/migration and
   generation-atomic updater/rollback work under owner-controlled admission.

**Until all gates complete, no public VIDEO release or production migration.**
