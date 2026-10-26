# AKULA-RELEASE-LOCK-01 — isolate the inherited Aquilum release

> **Historical scope / Windows status (2026-10-22):** This document
> describes a pre-release research stage and is NOT the current Windows
> updater gate. Akula v0.3.9 and v0.3.10 are already independently signed
> and published; the installed Windows release channel now uses the pinned
> Akula key and protected manual NSIS workflow. See
> [AKULA-VERSIONING-01](./AKULA-VERSIONING-01.md) for the active contract.
> The inherited Aquilum release workflow remains HOLD; Linux/macOS
> publisher approval and automatic NSIS crash rollback remain out of scope.

**Status: HOLD / testing only.** Canonical product and release requirements:
[AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).

## Why the old workflow cannot be used

The old \`.github/workflows/release.yml\` built a package from the inherited
Aquilum installer identity, downloaded and upgraded Freaction/Aquilum v0.2.6,
wrote to its per-user installation and AppData, and built Linux packages named
\`aquilum\` from \`native/packaging/nfpm.yaml\`.

Those operations are incompatible with an **independent Akula product** and
must never be executed as an Akula release or a migration test without an
explicit, reviewed boundary.

RELEASE-LOCK-01 preserves the exact old YAML, unmodified, under
\`docs/release/legacy-aquilum-release.yml\` (not a GitHub Actions workflow).
The active \`.github/workflows/release.yml\` is a \`workflow_dispatch\`
**fail-closed marker**: its only job emits an error and exits nonzero.
The new static CI policy asserts that the active workflow is read-only and
has no executable packaging or publishing jobs. It checks that the disabled
Akula updater still has no publisher public key.

## Active candidate testing (not release publication)

The separate \`.github/workflows/windows-beta.yml\` uses Akula-owned Windows
NSIS identity, a local-only \`--features installed\` executable, real
OS keyboard/edit/save/reopen, AccessKit UIA/keyboard checks and uninstall
that preserves the synthetic note and upstream Aquilum sentinels. Its upload
is a 14-day CI **candidate**, not a signed or published release.

[PR #366](https://github.com/Lalalalendia/Akula/pull/366) independently
tracks pinned Windows candidate checkout and SHA-256 provenance: **do not
implement a second manifest in RELEASE-LOCK-01.**

## Next release deliverables (independent acceptance)

1. **Akula Linux identity:** separate app ID, executable name, desktop file,
   Debian/RPM package name, install paths and icons; never overwrite or
   upgrade \`aquilum\`. Test AppImage/DEB/RPM on clean target distros and
   with Aquilum installed alongside Akula.
2. **Pinned Windows/Linux candidate source:** identical source commit and
   \`Cargo.lock\`, exact binary/package digests, install/edit/reopen/restart/
   uninstall smoke on each platform, fail-closed if any job is missing.
3. **Publisher trust and rollback:** Akula-owned Minisign key provisioned
   *outside* Git, verified signatures/hashes, fail-closed updater endpoint,
   Akula beta-v1 → Akula beta-v2 then interrupted upgrade/rollback tests.
   A hash printed next to a downloadable package does not authenticate it.
4. **Release approval:** real Windows 10/11 and Linux acceptance, local
   file-custody/recovery including interruption tests, evidence recorded at
   exact commit. Only then replace the HOLD marker with a gated, explicitly
   authorized Akula release pipeline.

**Never** enable old Freaction/Aquilum upgrade semantics, reuse Aquilum
signing keys, enable Sync destructive apply, publish artifacts automatically,
or unfreeze macOS (MAC-HOLD-01) as part of this step.
