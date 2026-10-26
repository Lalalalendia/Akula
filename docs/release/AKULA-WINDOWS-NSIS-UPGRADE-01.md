# RELEASE-WINDOWS-NSIS-UPGRADE-01 — Akula Windows update compatibility

> **Historical scope / Windows status (2026-10-22):** This document
> describes a pre-release research stage and is NOT the current Windows
> updater gate. Akula v0.3.9 and v0.3.10 are already independently signed
> and published; the installed Windows release channel now uses the pinned
> Akula key and protected manual NSIS workflow. See
> [AKULA-VERSIONING-01](./AKULA-VERSIONING-01.md) for the active contract.
> The inherited Aquilum release workflow remains HOLD; Linux/macOS
> publisher approval and automatic NSIS crash rollback remain out of scope.

**Status: Draft real two-version Windows CI acceptance. No Akula publisher signing root and no public auto-update activation.**

## Why Aquilum updates but Akula does not

Inspected exact upstream `Freaction/Aquilum` files on 2026-10-21:

- Upstream `aquilum-app/native/src/updater/release.rs` embeds a Minisign public key and reads `Freaction/Aquilum/releases/latest/download/latest.json`.
- Upstream `release.yml` produces a detached signature for Windows NSIS, verifies release signatures, generates `latest.json` and tests a **genuine installed Windows upgrade** from Aquilum 0.2.6 through `/S /R /UPDATE /ARGS`.
- Akula's Windows `apply.rs` retains those same NSIS invocation flags and its own `Akula-setup.exe` staging path, but `release.rs` points to `Lalalalendia/Akula` with an **empty** pinned public key and `channel_ready() == false`. `updater::available()` refuses to start the network check. The Akula repository has no published Release.
- Akula's existing Windows beta only tests **fresh installation, editor save/restart, and uninstall**. It never installed a second Akula version over the first, so nobody had proven NSIS update compatibility after the Aquilum fork.

This is not an NSIS implementation bug proven yet: the absence of an Akula signed release is a confirmed sufficient cause of the missing auto-update.

## What this PR actually executes

Only on a disposable GitHub-hosted Windows runner and only for the precisely named PR:

1. Build and test native 0.3.9 Akula, package one Akula NSIS installer, install/edit/restart/uninstall with preserved Aquilum sentinel data through the existing `test-windows-beta.ps1`.
2. Save the exact v1 installed executable SHA-256. In the same disposable runner, patch only native Cargo package version, its exact `Cargo.lock` record, and Akula-owned packaging JSON to 0.3.10. Run a distinct `cargo build --release --locked` and package the new, non-published NSIS installer. **Restore the original tracked checkout regardless of result**.
3. Require two different compiled native SHA-256 digests and two actual Akula installer packages, with source and digest provenance.
4. Install 0.3.9; open its real editor window on a synthetic Markdown vault.
5. Invoke 0.3.10 installer with the exact **upstream Windows updater** flags `/S /R /UPDATE /ARGS` while v1 is present. Check HKCU installed `DisplayVersion`, exactly one Akula uninstall identity, exact native v2 SHA-256, real GUI window and unchanged Markdown/Aquilum data. A successful subprocess exit without binary and GUI identity **does not pass**.
6. Simulate a failed v2 by terminating its GUI. Invoke the real v1 installer again with the same NSIS flags to test **operator-controlled recovery**. Check v1 exact bytes, visible v1 GUI and untouched Markdown/Aquilum witness files. This does **not** claim automatic production rollback after power failure.
7. Close the GUI, uninstall and check Akula user AppData and local notes remain, with existing upstream Aquilum paths and registry untouched.
8. Require exact `AKULA_WINDOWS_NSIS_TWO_VERSION_PASS install=1 v1_to_v2=1 recovery_v1=1 real_gui=1 data_untouched=1` from the executing PowerShell script. An unsigned v2 CI artifact is explicitly named `NOT-RELEASE` and expires after three days.

## What must happen before a user can click Update

The application cannot safely accept a downloaded executable without its own pinned trust root. The following are **P0 dependencies** for *working in-app Windows automatic updates*, not optional polish:

- Generate and protect a dedicated Akula Minisign private key **outside the source repository**, publish only its public key in a reviewed Windows client build, and use a protected release workflow to sign exact artifact and version/source metadata. Never use Freaction/Aquilum's private key.
- Publish real Akula-owned versioned NSIS packages plus signed manifest and enable channel discovery only after live signature, version, identity and anti-replay checks. A plain `latest.json` on HTTPS is **not** a signed version assertion.
- Wire a sealed `SignedCandidate` and exact installed source/version check into the Windows `apply` path, including stable staging and recovery. No raw unchecked `Vec<u8>` should authorize a user-file mutation.
- Test automated Windows update 0.3.9→0.3.10 with real user-facing UI, interrupted install/relaunch, current-version floor and rollback on Windows 10/11.
- Protect the existing data/editor/S3 and Aquilum identities, and require physical acceptance before public auto-update rollout.

**Never turn on `channel_ready()` merely to make the Update control visible.**

Evidence: [upstream Aquilum Windows updater](https://github.com/Freaction/Aquilum/blob/main/aquilum-app/native/src/updater/apply.rs), [Akula Windows beta](https://github.com/Lalalalendia/Akula/blob/main/.github/workflows/windows-beta.yml), [Notion master specification](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).
