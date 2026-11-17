# VIDEO-18b-11 — real five-EXE protected Program Files NSIS (non-release)

**Scope: Windows 2022 hosted pull-request runner only, unsigned, synthetic data, no product release.**

## Objective

Two earlier experiments cover separate boundaries:

- VIDEO-17 [#978](https://github.com/Lalalalendia/Akula/pull/978): genuine **real five-EXE NSIS**, device-local download, restart/GUI playback, clean uninstall — but installed under the writable per-user location, not a protected root.
- VIDEO-18b-03 [#984](https://github.com/Lalalalendia/Akula/pull/984): genuine **elevated protected Program Files NSIS**, 11 real non-admin write/delete/rename/replace/junction tests and uninstall — but containing **inert five-file markers**, not actual EXEs.
- VIDEO-18b-04 / 05 [#985](https://github.com/Lalalalendia/Akula/pull/985), [#987](https://github.com/Lalalalendia/Akula/pull/987): compiled Akula/Shiva/Rust mux, pinned genuine upstream yt-dlp/QuickJS, SHA-256 and five PE identities — but no protected install.

VIDEO-18b-11 joins **real binaries** and **protected root** in one reproducible host-only acceptance, without replacing the ordinary shipping installer.

## Exact non-release workflow

1. Checkout exact PR SHA with nonpersistent GitHub credentials. Checkout exact Shiva source commit and verify tracked lock; compile Shiva and Rust mux `--locked`; obtain pinned yt-dlp/QuickJS by pinned size/SHA; build experimental Akula from exact HEAD.
2. Run the unchanged VIDEO-18b-04 real-five custody witness. New script checks five files are actual PE/MZ, located only under isolated runner checkout/temp, with byte-identical stage hashes and the exact pinned mux/upstream references.
3. Install pinned NSIS 3.11.0 and build an **isolated** temporary admin-required NSIS from the five actual files. Destination is a randomly named `$PROGRAMFILES64\AkulaVideo18bRealNsisLab-<GUID>`, not stock Akula, Aquilum, HKCU, LocalAppData, or the signed candidate. It creates **no product registry identity**.
4. Verify genuine Program Files install, all five installed PE SHA-256 values, no reparse targets, and an actual NSIS uninstaller.
5. Create a disposable non-admin Windows account. Impersonate its Windows token and require 11 actual denied mutations: overwrite each of five real EXEs, delete, rename, helper injection, protected parent rename, child-directory creation, uninstaller overwrite. Non-admin must still read EXEs and write to an independent disposable local-media root.
6. Rehash all five actual EXEs after attack attempts, silently uninstall only the GUID-named protected NSIS, verify protected root removal and local-media preservation, delete disposable account/media/stage. No real EXEs or NSIS are uploaded; only the preexisting source-custody JSON report is retained briefly.

## Acceptance and limitations

- [ ] Source-contract negatives detect per-user root, unelevated NSIS, missing fifth binary, missing real Windows impersonation and lost source trigger.
- [ ] Exact-head Windows builds genuine five EXEs and accepts the existing custody proof.
- [ ] Real NSIS installs into protected root; **11 of 11** unprivileged mutations denied; local media remains writable; uninstall succeeds without deleting media.
- [ ] Application checks and no-growth policy successful.
- [ ] No production signing, public installer, registry migration, release, deployment, updater or default-feature modification.

This checks ACL-backed resistance to a **standard, unprivileged local user**. It does not prevent a local administrator, compromised kernel, signed updater compromise, or all same-user threats. It is **not** an integrated GUI acceptance on the newly protected root (the earlier genuine GUI E2E exercised a different install), and it does not implement generation-based update/rollback. These remain independent tests before any product release.

**Expected next engineering step:** protected installed GUI smoke using the real, isolated protected NSIS; then explicit installer identity/migration decision and crash-safe five-file updater journal.
